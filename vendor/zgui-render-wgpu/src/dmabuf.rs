//! Vulkan interop for DMA-BUFs handed off in GENERAL by a foreign queue. Only advertised,
//! importable DRM layouts are accepted. The wgpu tracker always sees the image in RESOURCE;
//! explicit acquire/release transfers bracket that lifetime on the same submission queue.

use std::{
    fmt,
    os::fd::{AsRawFd, BorrowedFd, IntoRawFd},
    sync::Arc,
};

use ash::vk;
use zgui_geom::{Device, Size};
use zgui_scene::ExternalTextureId;

use crate::{FrameTexture, Gpu, Nv12Target, WgpuRenderer};

pub(crate) const FOREIGN: &std::ffi::CStr = ash::ext::queue_family_foreign::NAME;
const DMA_BUF: vk::ExternalMemoryHandleTypeFlags = vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT;
const SAMPLED: vk::ImageUsageFlags = vk::ImageUsageFlags::SAMPLED;
const TILING: vk::ImageTiling = vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT;

/// DRM frame sample order and, for studio-range NV12, the RGB conversion matrix.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DmabufFormat {
    /// DRM_FORMAT_ABGR8888, sRGB straight alpha.
    Rgba8,
    /// DRM_FORMAT_ARGB8888, sRGB straight alpha.
    Bgra8,
    /// DRM_FORMAT_NV12, using the BT.601 coefficients.
    Nv12Bt601,
    /// DRM_FORMAT_NV12, using the BT.709 coefficients.
    Nv12Bt709,
}

impl DmabufFormat {
    fn format(self) -> (vk::Format, wgpu::TextureFormat, usize) {
        match self {
            Self::Rgba8 => (
                vk::Format::R8G8B8A8_UNORM,
                wgpu::TextureFormat::Rgba8Unorm,
                1,
            ),
            Self::Bgra8 => (
                vk::Format::B8G8R8A8_UNORM,
                wgpu::TextureFormat::Bgra8Unorm,
                1,
            ),
            Self::Nv12Bt601 | Self::Nv12Bt709 => (
                vk::Format::G8_B8R8_2PLANE_420_UNORM,
                wgpu::TextureFormat::NV12,
                2,
            ),
        }
    }

    fn index(self) -> usize {
        match self {
            Self::Rgba8 => 0,
            Self::Bgra8 => 1,
            Self::Nv12Bt601 | Self::Nv12Bt709 => 2,
        }
    }
}

/// A memory plane in the explicitly described DRM layout.
#[derive(Clone, Copy, Debug)]
pub struct DmabufPlane<'a> {
    /// The producer's descriptor, which remains owned by the caller.
    pub fd: BorrowedFd<'a>,
    /// Offset in that descriptor.
    pub offset: u64,
    /// Pitch in bytes.
    pub stride: u64,
}

/// DMA-BUF frame transferred in GENERAL to `VK_QUEUE_FAMILY_FOREIGN_EXT`.
/// The producer must not write it again until the release callback runs.
#[derive(Clone, Copy, Debug)]
pub struct Dmabuf<'a> {
    /// Frame extent in pixels.
    pub size: Size<u32, Device>,
    /// DRM image format and colour matrix.
    pub format: DmabufFormat,
    /// Explicit DRM modifier (including 0 for linear).
    pub modifier: u64,
    /// One descriptor may back multiple planes; plane order follows the DRM format.
    pub planes: &'a [DmabufPlane<'a>],
}

/// A rejected DMA-BUF import. The caller still owns its original descriptors.
#[derive(Debug)]
pub enum DmabufError {
    /// No compatible device or supported explicit format/modifier.
    Unsupported,
    /// The metadata, descriptor type or extent was incompatible with the image.
    InvalidLayout,
    /// The implicit producer fence has not signalled yet.
    NotReady,
    /// A descriptor could not be queried or duplicated.
    Io(std::io::Error),
    /// An import operation was refused by Vulkan.
    Vulkan(vk::Result),
}

impl fmt::Display for DmabufError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported => f.write_str("unsupported DMA-BUF adapter or format/modifier"),
            Self::InvalidLayout => f.write_str("invalid DMA-BUF descriptor or layout"),
            Self::NotReady => f.write_str("DMA-BUF writes are not complete"),
            Self::Io(error) => write!(f, "DMA-BUF descriptor: {error}"),
            Self::Vulkan(error) => write!(f, "DMA-BUF Vulkan import: {error}"),
        }
    }
}

impl std::error::Error for DmabufError {}
impl From<vk::Result> for DmabufError {
    fn from(value: vk::Result) -> Self {
        Self::Vulkan(value)
    }
}
impl From<std::io::Error> for DmabufError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

/// A device-owned imported frame that can be attached to multiple renderers on that device.
#[derive(Clone)]
pub struct ImportedFrame(Arc<Imported>);

impl fmt::Debug for ImportedFrame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImportedFrame")
            .field("size", &self.size())
            .finish_non_exhaustive()
    }
}

impl ImportedFrame {
    /// The imported image's extent in pixels.
    pub fn size(&self) -> Size<u32, Device> {
        self.0.size
    }
}

struct Imported {
    gpu: Arc<Gpu>,
    size: Size<u32, Device>,
    image: vk::Image,
    family: u32,
    texture: wgpu::Texture,
    converted: Option<wgpu::Texture>,
    release: Option<wgpu::hal::DropCallback>,
}

impl Drop for Imported {
    fn drop(&mut self) {
        let release = self
            .release
            .take()
            .expect("an imported frame owns one producer credit");
        if !self.gpu.loss().is_lost() {
            transfer(
                &self.gpu,
                &self.texture,
                self.image,
                self.family,
                Some(release),
            );
        } else {
            self.texture.destroy();
            release();
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Modifier {
    id: u64,
    disjoint: bool,
}

/// Coalesces completion polling independently of whether a window can draw another frame.
/// The worker owns a wgpu device handle, not the Gpu that owns the wake sender.
#[derive(Debug, Default)]
pub(crate) struct Retirement(std::sync::OnceLock<Option<std::sync::mpsc::SyncSender<()>>>);

impl Retirement {
    fn wake(&self, gpu: &Gpu) {
        use std::sync::mpsc::{self, TrySendError};
        let wake = self.0.get_or_init(|| {
            let (wake, requests) = mpsc::sync_channel(1);
            let device = gpu.device().clone();
            let queue = gpu.queue().clone();
            match std::thread::Builder::new()
                .name("zgui-frame-retirement".into())
                .spawn(move || {
                    // Keep the queue registered with wgpu until after the last scheduled poll.
                    let _queue = queue;
                    while requests.recv().is_ok() {
                        let _ = device.poll(wgpu::PollType::wait_indefinitely());
                        let _ = device.poll(wgpu::PollType::Poll);
                    }
                }) {
                Ok(_) => Some(wake),
                Err(error) => {
                    tracing::warn!(%error, "could not start the frame-retirement worker");
                    None
                }
            }
        });
        if !wake
            .as_ref()
            .is_some_and(|wake| matches!(wake.try_send(()), Ok(()) | Err(TrySendError::Full(()))))
        {
            gpu.wait();
        }
    }
}

impl Gpu {
    /// Whether this device supports at least one hardware DMA-BUF frame layout.
    pub fn dmabuf_supported(&self) -> bool {
        self.drm_modifiers().iter().any(|m| !m.is_empty())
    }

    /// Explicit DRM modifiers this device can import for `format`.
    pub fn dmabuf_modifiers(&self, format: DmabufFormat) -> Vec<u64> {
        self.drm_modifiers()[format.index()]
            .iter()
            .map(|m| m.id)
            .collect()
    }

    fn drm_modifiers(&self) -> &[Vec<Modifier>; 3] {
        self.drm_modifiers.get_or_init(|| {
            [
                DmabufFormat::Rgba8,
                DmabufFormat::Bgra8,
                DmabufFormat::Nv12Bt601,
            ]
            .map(|format| modifiers(self, format))
        })
    }
}

impl WgpuRenderer {
    /// Whether this renderer can import DMA-BUFs on its active device.
    pub fn dmabuf_supported(&self) -> bool {
        self.gpu().dmabuf_supported()
    }

    /// Imports a producer frame without reading its pixels on the CPU. The callback runs
    /// after the last attachment is retired, the release barrier completes, and Vulkan
    /// no longer owns the image. On rejection the callback is dropped without calling it.
    ///
    /// Use a separate `nv12_target` per stream. Successive NV12 frames on the same
    /// device and extent share its RGB texture, with writes ordered on the GPU queue.
    ///
    /// # Errors
    /// Refuses incompatible formats, invalid descriptors and unsignalled DMA-BUF fences.
    pub fn import_dmabuf(
        &self,
        frame: Dmabuf<'_>,
        nv12_target: &mut Nv12Target,
        release: impl FnOnce() + Send + Sync + 'static,
    ) -> Result<ImportedFrame, DmabufError> {
        import(self.gpu(), frame, nv12_target, Box::new(release))
    }

    /// Registers an imported texture on a renderer sharing its device. Returns no binding
    /// on a different device, including after renderer recovery.
    pub fn attach_imported_frame(
        &mut self,
        id: ExternalTextureId,
        frame: &ImportedFrame,
    ) -> Option<FrameTexture> {
        if !Arc::ptr_eq(self.gpu(), &frame.0.gpu) {
            return None;
        }
        let texture = frame
            .0
            .converted
            .as_ref()
            .unwrap_or(&frame.0.texture)
            .clone();
        Some(self.bind_frame(id, texture, Some(frame.clone())))
    }
}

fn vulkan(gpu: &Gpu) -> Option<impl std::ops::Deref<Target = wgpu::hal::vulkan::Device> + '_> {
    if gpu.adapter().get_info().device_type == wgpu::DeviceType::Cpu {
        return None;
    }
    // SAFETY: the guard is borrowed; no handle is destroyed or used after this device dies.
    let hal = unsafe { gpu.device().as_hal::<wgpu::hal::api::Vulkan>() }?;
    let enabled = hal.enabled_device_extensions();
    [
        FOREIGN,
        ash::ext::image_drm_format_modifier::NAME,
        ash::ext::external_memory_dma_buf::NAME,
        ash::khr::external_memory_fd::NAME,
    ]
    .iter()
    .all(|name| enabled.contains(name))
    .then_some(hal)
}

fn flags(format: DmabufFormat, disjoint: bool) -> vk::ImageCreateFlags {
    let mut flags = vk::ImageCreateFlags::empty();
    if format.index() == 2 {
        flags |= vk::ImageCreateFlags::MUTABLE_FORMAT | vk::ImageCreateFlags::EXTENDED_USAGE;
    }
    if disjoint {
        flags |= vk::ImageCreateFlags::DISJOINT;
    }
    flags
}

fn image_properties(
    device: &wgpu::hal::vulkan::Device,
    format: DmabufFormat,
    modifier: u64,
    disjoint: bool,
) -> Option<vk::ImageFormatProperties> {
    let mut drm = vk::PhysicalDeviceImageDrmFormatModifierInfoEXT::default()
        .drm_format_modifier(modifier)
        .sharing_mode(vk::SharingMode::EXCLUSIVE);
    let mut external = vk::PhysicalDeviceExternalImageFormatInfo::default().handle_type(DMA_BUF);
    let input = vk::PhysicalDeviceImageFormatInfo2::default()
        .format(format.format().0)
        .ty(vk::ImageType::TYPE_2D)
        .tiling(TILING)
        .usage(SAMPLED)
        .flags(flags(format, disjoint))
        .push_next(&mut drm)
        .push_next(&mut external);
    let mut external = vk::ExternalImageFormatProperties::default();
    let mut output = vk::ImageFormatProperties2::default().push_next(&mut external);
    // SAFETY: all pNext structures outlive the query and the physical handle belongs to this instance.
    unsafe {
        device
            .shared_instance()
            .raw_instance()
            .get_physical_device_image_format_properties2(
                device.raw_physical_device(),
                &input,
                &mut output,
            )
    }
    .ok()?;
    let props = output.image_format_properties;
    let features = external.external_memory_properties.external_memory_features;
    (features.contains(vk::ExternalMemoryFeatureFlags::IMPORTABLE)
        && (!disjoint || !features.contains(vk::ExternalMemoryFeatureFlags::DEDICATED_ONLY)))
    .then_some(props)
}

fn modifiers(gpu: &Gpu, format: DmabufFormat) -> Vec<Modifier> {
    let Some(hal) = vulkan(gpu) else {
        return Vec::new();
    };
    let (raw, _, planes) = format.format();
    if planes == 2
        && !gpu
            .device()
            .features()
            .contains(wgpu::Features::TEXTURE_FORMAT_NV12)
    {
        return Vec::new();
    }
    let query = |list: &mut vk::DrmFormatModifierPropertiesListEXT<'_>| {
        // SAFETY: the list points to writable storage of the supplied capacity or is null for a count query.
        unsafe {
            hal.shared_instance()
                .raw_instance()
                .get_physical_device_format_properties2(
                    hal.raw_physical_device(),
                    raw,
                    &mut vk::FormatProperties2::default().push_next(list),
                )
        };
    };
    let mut list = vk::DrmFormatModifierPropertiesListEXT::default();
    query(&mut list);
    let mut props = vec![
        vk::DrmFormatModifierPropertiesEXT::default();
        list.drm_format_modifier_count as usize
    ];
    let mut list = vk::DrmFormatModifierPropertiesListEXT::default()
        .drm_format_modifier_properties(&mut props);
    query(&mut list);
    let count = list.drm_format_modifier_count as usize;
    let features = vk::FormatFeatureFlags::SAMPLED_IMAGE
        | if planes == 1 {
            vk::FormatFeatureFlags::SAMPLED_IMAGE_FILTER_LINEAR
        } else {
            vk::FormatFeatureFlags::empty()
        };
    props[..count]
        .iter()
        .filter_map(|p| {
            let id = p.drm_format_modifier;
            (p.drm_format_modifier_plane_count as usize == planes
                && p.drm_format_modifier_tiling_features.contains(features)
                && image_properties(&hal, format, id, false).is_some())
            .then(|| Modifier {
                id,
                disjoint: planes == 2
                    && p.drm_format_modifier_tiling_features
                        .contains(vk::FormatFeatureFlags::DISJOINT)
                    && image_properties(&hal, format, id, true).is_some(),
            })
        })
        .collect()
}

struct Allocation {
    raw: ash::Device,
    image: vk::Image,
    memory: [vk::DeviceMemory; 2],
}

impl Drop for Allocation {
    fn drop(&mut self) {
        // SAFETY: this owns the VkImage and each successful allocation. Before registration no
        // work uses them; afterwards wgpu invokes the callback only after all submissions retire.
        unsafe {
            self.raw.destroy_image(self.image, None);
            for memory in self
                .memory
                .into_iter()
                .filter(|m| *m != vk::DeviceMemory::null())
            {
                self.raw.free_memory(memory, None);
            }
        }
    }
}

fn import(
    gpu: &Arc<Gpu>,
    frame: Dmabuf<'_>,
    nv12_target: &mut Nv12Target,
    release: wgpu::hal::DropCallback,
) -> Result<ImportedFrame, DmabufError> {
    let device = vulkan(gpu).ok_or(DmabufError::Unsupported)?;
    let (vk_format, format, count) = frame.format.format();
    let size = frame.size;
    let limit = gpu.device().limits().max_texture_dimension_2d.min(8192);
    if frame.planes.len() != count
        || size.width == 0
        || size.height == 0
        || size.width > limit
        || size.height > limit
        || (count == 2 && (!size.width.is_multiple_of(2) || !size.height.is_multiple_of(2)))
    {
        return Err(DmabufError::InvalidLayout);
    }
    let disjoint = count == 2 && frame.planes[0].fd.as_raw_fd() != frame.planes[1].fd.as_raw_fd();
    if !gpu.drm_modifiers()[frame.format.index()]
        .iter()
        .any(|m| m.id == frame.modifier && (!disjoint || m.disjoint))
    {
        return Err(DmabufError::Unsupported);
    }
    let max = image_properties(&device, frame.format, frame.modifier, disjoint)
        .ok_or(DmabufError::Unsupported)?
        .max_extent;
    if size.width > max.width || size.height > max.height {
        return Err(DmabufError::InvalidLayout);
    }
    let mut allocation_sizes = [0; 2];
    let mut layouts = [vk::SubresourceLayout::default(); 2];
    for (i, plane) in frame.planes.iter().enumerate() {
        let fd_index = if disjoint { i } else { 0 };
        if i == 0 || disjoint {
            allocation_sizes[fd_index] = ready_size(plane.fd)?;
        }
        let rows = if i == 0 { size.height } else { size.height / 2 };
        let width = if count == 1 {
            size.width * 4
        } else {
            size.width
        };
        if plane.stride < u64::from(width)
            || plane
                .offset
                .checked_add(
                    plane
                        .stride
                        .checked_mul(rows.into())
                        .ok_or(DmabufError::InvalidLayout)?,
                )
                .is_none_or(|end| end > allocation_sizes[fd_index])
        {
            return Err(DmabufError::InvalidLayout);
        }
        layouts[i].offset = plane.offset;
        layouts[i].row_pitch = plane.stride;
    }
    let mut external = vk::ExternalMemoryImageCreateInfo::default().handle_types(DMA_BUF);
    let mut drm = vk::ImageDrmFormatModifierExplicitCreateInfoEXT::default()
        .drm_format_modifier(frame.modifier)
        .plane_layouts(&layouts[..count]);
    let raw = device.raw_device();
    // SAFETY: the queried format/modifier supports exactly these planes, flags and usage. Vulkan
    // validates explicit pitches with the driver; the layout slice lives through creation.
    let image = unsafe {
        raw.create_image(
            &vk::ImageCreateInfo::default()
                .push_next(&mut external)
                .push_next(&mut drm)
                .flags(flags(frame.format, disjoint))
                .image_type(vk::ImageType::TYPE_2D)
                .format(vk_format)
                .extent(vk::Extent3D {
                    width: size.width,
                    height: size.height,
                    depth: 1,
                })
                .mip_levels(1)
                .array_layers(1)
                .samples(vk::SampleCountFlags::TYPE_1)
                .tiling(TILING)
                .usage(SAMPLED),
            None,
        )
    }?;
    let mut allocation = Allocation {
        raw: raw.clone(),
        image,
        memory: [vk::DeviceMemory::null(); 2],
    };
    bind(
        &device,
        &mut allocation,
        frame.planes,
        allocation_sizes,
        disjoint,
    )?;
    let extent = crate::frame_texture::extent(size);
    let desc = wgpu::TextureDescriptor {
        label: Some("zgui.dma-buf"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    };
    let hal = wgpu::hal::TextureDescriptor {
        label: desc.label,
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: desc.dimension,
        format,
        usage: wgpu::TextureUses::RESOURCE,
        memory_flags: wgpu::hal::MemoryFlags::empty(),
        view_formats: Vec::new(),
    };
    let destroy = Box::new(move || drop(allocation));
    // SAFETY: the descriptor exactly matches the image and the bound memory is owned by `destroy`.
    let texture = unsafe {
        device.texture_from_raw(
            image,
            &hal,
            Some(destroy),
            wgpu::hal::vulkan::TextureMemory::External,
        )
    };
    let family = device.queue_family_index();
    drop(device);
    // SAFETY: the image was created on this device, has defined producer bytes, and our first
    // command acquires it and establishes RESOURCE before any wgpu command can sample it.
    let texture = unsafe {
        gpu.device()
            .create_texture_from_hal::<wgpu::hal::api::Vulkan>(
                texture,
                &desc,
                wgpu::TextureUses::RESOURCE,
            )
    };
    transfer(gpu, &texture, image, family, None);
    let mut imported = Imported {
        gpu: gpu.clone(),
        size,
        image,
        family,
        texture,
        converted: None,
        release: Some(release),
    };
    if count == 2 {
        imported.converted = Some(nv12_target.convert(
            gpu,
            &imported.texture,
            frame.format == DmabufFormat::Nv12Bt709,
        ));
    }
    Ok(ImportedFrame(Arc::new(imported)))
}

fn bind(
    device: &wgpu::hal::vulkan::Device,
    allocation: &mut Allocation,
    planes: &[DmabufPlane<'_>],
    sizes: [u64; 2],
    disjoint: bool,
) -> Result<(), DmabufError> {
    let raw = device.raw_device();
    let loader =
        ash::khr::external_memory_fd::Device::new(device.shared_instance().raw_instance(), raw);
    let aspects = [
        vk::ImageAspectFlags::MEMORY_PLANE_0_EXT,
        vk::ImageAspectFlags::MEMORY_PLANE_1_EXT,
    ];
    for i in 0..if disjoint { 2 } else { 1 } {
        let mut plane = vk::ImagePlaneMemoryRequirementsInfo::default().plane_aspect(aspects[i]);
        let mut info = vk::ImageMemoryRequirementsInfo2::default().image(allocation.image);
        if disjoint {
            info = info.push_next(&mut plane);
        }
        let mut requirements = vk::MemoryRequirements2::default();
        let fd = planes[i].fd.try_clone_to_owned()?;
        let mut fd_props = vk::MemoryFdPropertiesKHR::default();
        // SAFETY: the fd was validated as a DMA-BUF and retained through both queries. For a
        // disjoint image the queried memory aspect is one of the advertised modifier planes.
        unsafe {
            raw.get_image_memory_requirements2(&info, &mut requirements);
            loader.get_memory_fd_properties(DMA_BUF, fd.as_raw_fd(), &mut fd_props)?;
        }
        let requirements = requirements.memory_requirements;
        let mask = requirements.memory_type_bits & fd_props.memory_type_bits;
        if mask == 0 || sizes[i] < requirements.size {
            return Err(DmabufError::InvalidLayout);
        }
        let mut import = vk::ImportMemoryFdInfoKHR::default()
            .fd(fd.as_raw_fd())
            .handle_type(DMA_BUF);
        let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(allocation.image);
        let mut info = vk::MemoryAllocateInfo::default()
            .allocation_size(sizes[i])
            .memory_type_index(mask.trailing_zeros())
            .push_next(&mut import);
        if !disjoint {
            info = info.push_next(&mut dedicated);
        }
        // SAFETY: fd, memory type, size and dedicated image were checked against the driver.
        // The owned duplicate is disarmed only on success, when Vulkan takes ownership.
        allocation.memory[i] = unsafe { raw.allocate_memory(&info, None) }?;
        let _ = fd.into_raw_fd();
    }
    if disjoint {
        let mut first = vk::BindImagePlaneMemoryInfo::default().plane_aspect(aspects[0]);
        let mut second = vk::BindImagePlaneMemoryInfo::default().plane_aspect(aspects[1]);
        let binds = [
            vk::BindImageMemoryInfo::default()
                .image(allocation.image)
                .memory(allocation.memory[0])
                .push_next(&mut first),
            vk::BindImageMemoryInfo::default()
                .image(allocation.image)
                .memory(allocation.memory[1])
                .push_next(&mut second),
        ];
        // SAFETY: both planes have independent compatible allocations at offset zero.
        unsafe { raw.bind_image_memory2(&binds) }?;
    } else {
        // SAFETY: this dedicated allocation was created for this image and meets its requirements.
        unsafe { raw.bind_image_memory(allocation.image, allocation.memory[0], 0) }?;
    }
    Ok(())
}

fn ready_size(fd: BorrowedFd<'_>) -> Result<u64, DmabufError> {
    use rustix::{
        event::{PollFd, PollFlags, poll},
        fs::{SeekFrom, seek},
        ioctl,
    };
    use std::os::fd::{AsFd, FromRawFd, OwnedFd};
    #[repr(C)]
    struct ExportSync {
        flags: u32,
        fd: i32,
    }
    const EXPORT: ioctl::Opcode = ioctl::opcode::read_write::<ExportSync>(b'b', 2);
    let mut sync = ExportSync { flags: 1, fd: -1 }; // DMA_BUF_SYNC_READ waits for writers only.
    // SAFETY: DMA_BUF_IOCTL_EXPORT_SYNC_FILE reads flags and writes a new descriptor into this
    // exact Linux UAPI layout. The call also rejects ordinary files masquerading as DMA-BUFs.
    let fence = unsafe {
        ioctl::ioctl(fd, ioctl::Updater::<EXPORT, _>::new(&mut sync))
            .map_err(std::io::Error::from)?;
        OwnedFd::from_raw_fd(sync.fd)
    };
    let mut descriptors = [PollFd::new(&fence, PollFlags::IN)];
    let zero = rustix::event::Timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    poll(&mut descriptors, Some(&zero)).map_err(std::io::Error::from)?;
    if descriptors[0].revents().contains(PollFlags::ERR) {
        return Err(DmabufError::InvalidLayout);
    }
    if !descriptors[0].revents().contains(PollFlags::IN) {
        return Err(DmabufError::NotReady);
    }
    seek(fd.as_fd(), SeekFrom::End(0)).map_err(|err| DmabufError::Io(err.into()))
}

fn transfer(
    gpu: &Gpu,
    texture: &wgpu::Texture,
    image: vk::Image,
    family: u32,
    release: Option<wgpu::hal::DropCallback>,
) {
    let mut encoder = gpu
        .device()
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("zgui.dma-buf-ownership"),
        });
    // SAFETY: this encoder and image both belong to the held Vulkan device. The raw command
    // records only a barrier, never ends the encoder, and wgpu submits it in queue order.
    unsafe {
        let hal = gpu
            .device()
            .as_hal::<wgpu::hal::api::Vulkan>()
            .expect("DMA-BUF has a Vulkan device");
        encoder.as_hal_mut::<wgpu::hal::api::Vulkan, _, _>(|commands| {
            let commands = commands.expect("the DMA-BUF encoder uses Vulkan");
            let barrier = vk::ImageMemoryBarrier::default()
                .image(image)
                .subresource_range(
                    vk::ImageSubresourceRange::default()
                        .aspect_mask(vk::ImageAspectFlags::COLOR)
                        .level_count(1)
                        .layer_count(1),
                );
            let general = vk::ImageLayout::GENERAL;
            let read = vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL;
            let fragment = vk::PipelineStageFlags::FRAGMENT_SHADER;
            let (barrier, src, dst) = if release.is_none() {
                (
                    barrier
                        .old_layout(general)
                        .new_layout(read)
                        .src_queue_family_index(vk::QUEUE_FAMILY_FOREIGN_EXT)
                        .dst_queue_family_index(family)
                        .dst_access_mask(vk::AccessFlags::SHADER_READ),
                    vk::PipelineStageFlags::TOP_OF_PIPE,
                    fragment,
                )
            } else {
                (
                    barrier
                        .old_layout(read)
                        .new_layout(general)
                        .src_queue_family_index(family)
                        .dst_queue_family_index(vk::QUEUE_FAMILY_FOREIGN_EXT)
                        .src_access_mask(vk::AccessFlags::SHADER_READ),
                    fragment,
                    vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                )
            };
            hal.raw_device().cmd_pipeline_barrier(
                commands.raw_handle(),
                src,
                dst,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[barrier],
            );
        });
    }
    gpu.queue().submit([encoder.finish()]);
    // Raw encoding cannot name a wgpu texture in its resource tracker. Keep it alive through
    // this submission separately, including a frame retired before it was ever drawn.
    let until_complete = texture.clone();
    let retiring = release.is_some();
    gpu.queue().on_submitted_work_done(move || {
        if let Some(release) = release {
            until_complete.destroy();
            release();
        }
    });
    if retiring {
        gpu.frame_retirement.wake(gpu);
    }
}
