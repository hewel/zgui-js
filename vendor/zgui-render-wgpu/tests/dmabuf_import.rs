//! Public external-frame rendering on a real Vulkan export, including an explicit CPU-adapter result.
#![cfg(target_os = "linux")]
#![allow(unsafe_code)]

mod support;

use ash::vk;
use std::os::fd::{AsFd, FromRawFd, OwnedFd};
use zgui_geom::{DevicePx, Size, Vec2};
use zgui_render_wgpu::{Dmabuf, DmabufFormat, DmabufPlane, Gpu, wgpu};
use zgui_scene::{ClipLink, ExternalQuad, ExternalTextureId, Scene};

const WIDTH: u32 = 16;
const HEIGHT: u32 = 8;

#[test]
fn imported_vulkan_frames_draw_exact_clipped_rgb_pixels() {
    let mut renderer = support::plain_renderer()
        .expect("renderer test needs a working adapter, including software Vulkan");
    let gpu = renderer.gpu().clone();
    let info = gpu.adapter().get_info();
    let capable = info.backend == wgpu::Backend::Vulkan
        && info.device_type != wgpu::DeviceType::Cpu
        && gpu
            .adapter()
            .features()
            .contains(wgpu::Features::VULKAN_EXTERNAL_MEMORY_DMA_BUF);
    assert_eq!(renderer.dmabuf_supported(), capable, "{info:?}");
    if !capable {
        return;
    }

    for (format, vk_format) in [
        (DmabufFormat::Rgba8, vk::Format::R8G8B8A8_UNORM),
        (
            DmabufFormat::Nv12Bt601,
            vk::Format::G8_B8R8_2PLANE_420_UNORM,
        ),
    ] {
        assert!(
            gpu.dmabuf_modifiers(format).contains(&0),
            "linear modifier for {format:?}"
        );
        let export = export_red(&gpu, vk_format);
        let planes: Vec<_> = export
            .layouts
            .iter()
            .map(|layout| DmabufPlane {
                fd: export.fd.as_fd(),
                offset: layout.offset,
                stride: layout.row_pitch,
            })
            .collect();
        let (release, released) = std::sync::mpsc::channel();
        let imported = renderer
            .import_dmabuf(
                Dmabuf {
                    size: Size::new(WIDTH, HEIGHT),
                    format,
                    modifier: 0,
                    planes: &planes,
                },
                move || {
                    let _ = release.send(());
                },
            )
            .expect("import Vulkan export");
        let id = ExternalTextureId(42);
        let slot = renderer
            .attach_imported_frame(id, &imported)
            .expect("same renderer device");
        let mut scene = Scene::new();
        scene.begin_frame(Size::new(support::SIDE, support::SIDE));
        let clip = scene.clips.only(ClipLink::rounded(
            support::rect(12.0, 12.0, 84.0, 84.0),
            Vec2::splat(DevicePx(24.0)),
        ));
        scene.push_external(
            ExternalQuad::new(support::rect(0.0, 0.0, 108.0, 108.0), id).clipped(clip),
        );
        scene.finish(&zgui_bits::DamageSet::full());
        let pixels = support::present(&mut renderer, &scene);
        assert_eq!(pixels.rgba(50, 50), [255, 0, 0, 255], "{format:?}");
        assert_eq!(
            pixels.rgba(13, 13),
            [0, 0, 0, 0],
            "rounded clip for {format:?}"
        );
        assert!(
            matches!(
                released.try_recv(),
                Err(std::sync::mpsc::TryRecvError::Empty)
            ),
            "visible frame still owns its producer credit"
        );
        renderer.remove_frame(slot);
        drop(imported);
        released
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("retirement returns credit even without another draw or explicit device poll");
    }
}

struct Export {
    raw: ash::Device,
    image: vk::Image,
    memory: vk::DeviceMemory,
    fd: OwnedFd,
    layouts: Vec<vk::SubresourceLayout>,
}

impl Drop for Export {
    fn drop(&mut self) {
        // SAFETY: the producer waited for its own commands; the importer has independent Vulkan resources.
        unsafe {
            self.raw.destroy_image(self.image, None);
            self.raw.free_memory(self.memory, None);
        }
    }
}

fn export_red(gpu: &Gpu, format: vk::Format) -> Export {
    // SAFETY: the renderer and device outlive this function, and their handles are not destroyed here.
    let hal = unsafe { gpu.device().as_hal::<wgpu::hal::api::Vulkan>() }.unwrap();
    let raw = hal.raw_device();
    let instance = hal.shared_instance().raw_instance();
    let device = hal.raw_physical_device();
    let mut external = vk::ExternalMemoryImageCreateInfo::default()
        .handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
    let mut modifiers =
        vk::ImageDrmFormatModifierListCreateInfoEXT::default().drm_format_modifiers(&[0]);
    let nv12 = format == vk::Format::G8_B8R8_2PLANE_420_UNORM;
    let flags = if nv12 {
        vk::ImageCreateFlags::MUTABLE_FORMAT
    } else {
        vk::ImageCreateFlags::empty()
    };
    // SAFETY: all pNext structs and modifier arrays live through synchronous creation; formats are checked by the test above.
    let image = unsafe {
        raw.create_image(
            &vk::ImageCreateInfo::default()
                .push_next(&mut external)
                .push_next(&mut modifiers)
                .flags(flags)
                .image_type(vk::ImageType::TYPE_2D)
                .format(format)
                .extent(vk::Extent3D {
                    width: WIDTH,
                    height: HEIGHT,
                    depth: 1,
                })
                .mip_levels(1)
                .array_layers(1)
                .samples(vk::SampleCountFlags::TYPE_1)
                .tiling(vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT)
                .usage(vk::ImageUsageFlags::SAMPLED),
            None,
        )
    }
    .unwrap();
    // SAFETY: the same Vulkan device owns the image and the physical handle.
    let (requirements, types) = unsafe {
        (
            raw.get_image_memory_requirements(image),
            instance.get_physical_device_memory_properties(device),
        )
    };
    let memory_type = (0..types.memory_type_count)
        .find(|&i| {
            requirements.memory_type_bits & (1 << i) != 0
                && types.memory_types[i as usize].property_flags.contains(
                    vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
                )
        })
        .unwrap();
    let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(image);
    let mut external = vk::ExportMemoryAllocateInfo::default()
        .handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
    // SAFETY: allocation honors the image requirements and its supported memory type.
    let memory = unsafe {
        raw.allocate_memory(
            &vk::MemoryAllocateInfo::default()
                .push_next(&mut dedicated)
                .push_next(&mut external)
                .allocation_size(requirements.size)
                .memory_type_index(memory_type),
            None,
        )
    }
    .unwrap();
    // SAFETY: the dedicated allocation was created for this image and has adequate size.
    unsafe { raw.bind_image_memory(image, memory, 0) }.unwrap();
    let aspects = [
        vk::ImageAspectFlags::MEMORY_PLANE_0_EXT,
        vk::ImageAspectFlags::MEMORY_PLANE_1_EXT,
    ];
    let layouts: Vec<_> = aspects[..if nv12 { 2 } else { 1 }]
        .iter()
        .map(|&aspect| {
            // SAFETY: DRM modifier tiling names its memory planes with MEMORY_PLANE_N_EXT.
            unsafe {
                raw.get_image_subresource_layout(
                    image,
                    vk::ImageSubresource::default().aspect_mask(aspect),
                )
            }
        })
        .collect();

    // Initialize the image to GENERAL first, then write into the host coherent linear planes.
    transition(gpu, raw, &hal, image, false);
    // SAFETY: the selected memory type is host visible and coherent; the transition finished.
    let mapped =
        unsafe { raw.map_memory(memory, 0, requirements.size, vk::MemoryMapFlags::empty()) }
            .unwrap();
    // SAFETY: Vulkan returned a mapping of requirements.size bytes, with no outstanding GPU use.
    let bytes =
        unsafe { std::slice::from_raw_parts_mut(mapped.cast::<u8>(), requirements.size as usize) };
    for (plane, layout) in layouts.iter().enumerate() {
        let (rows, row_bytes, sample): (u32, u32, &[u8]) = match (nv12, plane) {
            (false, _) => (HEIGHT, WIDTH * 4, &[255, 0, 0, 255]),
            (true, 0) => (HEIGHT, WIDTH, &[81]),
            (true, _) => (HEIGHT / 2, WIDTH, &[90, 240]),
        };
        for row in 0..rows {
            let at = (layout.offset + u64::from(row) * layout.row_pitch) as usize;
            for out in bytes[at..at + row_bytes as usize].chunks_exact_mut(sample.len()) {
                out.copy_from_slice(sample);
            }
        }
    }
    // SAFETY: the mapping above is finished; no reference to it is used after unmapping.
    unsafe { raw.unmap_memory(memory) };
    transition(gpu, raw, &hal, image, true);
    // SAFETY: the memory was allocated with this export handle type; Vulkan returns a new owned FD.
    let fd = unsafe {
        ash::khr::external_memory_fd::Device::new(instance, raw)
            .get_memory_fd(
                &vk::MemoryGetFdInfoKHR::default()
                    .memory(memory)
                    .handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT),
            )
            .map(|fd| OwnedFd::from_raw_fd(fd))
    }
    .unwrap();
    Export {
        raw: raw.clone(),
        image,
        memory,
        fd,
        layouts,
    }
}

fn transition(
    gpu: &Gpu,
    raw: &ash::Device,
    hal: &wgpu::hal::vulkan::Device,
    image: vk::Image,
    release: bool,
) {
    gpu.wait();
    let family = hal.queue_family_index();
    // SAFETY: this test alone submits to this queue, waits for completion before destroying the pool,
    // and the bound image and its single subresource remain alive throughout.
    unsafe {
        let pool = raw
            .create_command_pool(
                &vk::CommandPoolCreateInfo::default().queue_family_index(family),
                None,
            )
            .unwrap();
        let commands = raw
            .allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(pool)
                    .command_buffer_count(1),
            )
            .unwrap();
        raw.begin_command_buffer(commands[0], &vk::CommandBufferBeginInfo::default())
            .unwrap();
        let mut barrier = vk::ImageMemoryBarrier::default()
            .image(image)
            .subresource_range(
                vk::ImageSubresourceRange::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .level_count(1)
                    .layer_count(1),
            )
            .new_layout(vk::ImageLayout::GENERAL);
        let (src, dst) = if release {
            barrier = barrier
                .old_layout(vk::ImageLayout::GENERAL)
                .src_queue_family_index(family)
                .dst_queue_family_index(vk::QUEUE_FAMILY_FOREIGN_EXT)
                .src_access_mask(vk::AccessFlags::HOST_WRITE);
            (
                vk::PipelineStageFlags::HOST,
                vk::PipelineStageFlags::BOTTOM_OF_PIPE,
            )
        } else {
            barrier = barrier
                .old_layout(vk::ImageLayout::UNDEFINED)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED);
            (
                vk::PipelineStageFlags::TOP_OF_PIPE,
                vk::PipelineStageFlags::HOST,
            )
        };
        raw.cmd_pipeline_barrier(
            commands[0],
            src,
            dst,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[barrier],
        );
        raw.end_command_buffer(commands[0]).unwrap();
        raw.queue_submit(
            hal.raw_queue(),
            &[vk::SubmitInfo::default().command_buffers(&commands)],
            vk::Fence::null(),
        )
        .unwrap();
        raw.queue_wait_idle(hal.raw_queue()).unwrap();
        raw.destroy_command_pool(pool, None);
    }
}
