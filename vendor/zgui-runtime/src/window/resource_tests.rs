//! Scheduling gates pause actual codec jobs, not fabricated readiness records.
use std::{sync::Arc, time::{Duration, Instant}};
use zgui_platform_headless::Harness;
use zgui_render::Renderer;
use zgui_view::{AttrName, BuildCx, ClassName, DomHandle, ElementName, IntoView, View};
use crate::{App, ImagePolicy, ImageState, Runtime};

fn raster() -> Arc<[u8]> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 512, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header().unwrap().write_image_data(&[255, 0, 0, 255].repeat(512)).unwrap();
    }
    bytes.into()
}

fn publish(app: &mut Harness<Runtime>) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        while !app.app_mut().windows_mut()[0].poll_image_barrier() {
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        app.settle(8);
        if app.app().windows()[0].image_resources().resources.iter().all(|resource| {
            !resource.visible || matches!(resource.state, ImageState::Ready | ImageState::Canceled)
        }) { return; }
        assert!(Instant::now() < deadline);
    }
}

#[test]
fn delayed_dpr_decode_keeps_old_attachment_pending_then_source_replacement_retires_it() {
    let handler = App::new().with_size(400.0, 300.0)
        .with_stylesheet("image { display:block; width:64px; height:16px }")
        .with_image_policy(ImagePolicy::package_only([
            ("wide.png".into(), raster()),
            ("replacement.png".into(), Arc::from(include_bytes!("../../tests/fixtures/red2x2.png").as_slice())),
        ]))
        .with_renderer(Box::new(|_, target| {
            let mut renderer = zgui_testkit_scene::CaptureRenderer::new();
            renderer.configure(target);
            Ok(Box::new(renderer))
        }))
        .into_handler(|cx: &mut BuildCx<'_>| {
            Box::new(zgui_elements::image().class("picture").src(Some(String::from("wide.png"))).into_view().build(cx))
        }).unwrap();
    let mut app = Harness::new(handler);
    app.settle(8);
    publish(&mut app);
    let original = app.app().windows()[0].image_resources().resources[0].clone();
    let release = app.app().windows()[0].images.hold_next_for_tests();
    app.deliver_to_first(zgui_platform::SurfaceEvent::ScaleFactorChanged {
        scale_factor: 2.0,
        size: zgui_geom::Size::new(zgui_geom::DevicePx(800.0), zgui_geom::DevicePx(600.0)),
    });
    app.settle(8);
    let time = app.now();
    let frames = app.frames_requested();
    for _ in 0..16 {
        assert!(!app.app_mut().windows_mut()[0].poll_image_barrier());
        let waiting = app.app().windows()[0].image_resources().resources[0].clone();
        assert_eq!(waiting.id, original.id);
        assert_eq!(waiting.state, ImageState::Loading);
        assert!(waiting.requested_variant > waiting.attached_variant);
    }
    assert_eq!(app.now(), time);
    assert_eq!(app.frames_requested(), frames);
    let window = &app.app().windows()[0];
    let node = window.nodes_with_class("picture")[0];
    DomHandle::from_rc(window.dom().clone()).set_attribute(node, AttrName::new("src"), Some("replacement.png"));
    window.request_frame();
    app.settle(8);
    let before_release = app.app().windows()[0].image_resources();
    assert!(before_release.resources.iter().any(|resource| resource.id == original.id && resource.state == ImageState::Canceled));
    release.send(()).unwrap();
    publish(&mut app);
    let ready = app.app().windows()[0].image_resources();
    let active = ready.resources.iter().find(|resource| resource.state == ImageState::Ready).unwrap();
    assert_eq!(active.source, "replacement.png");
    assert_ne!(active.id, original.id);
    assert!(active.attached_variant >= active.requested_variant);
    assert_eq!(app.now(), time);
    // Detachment follows the native document's frame-end retirement path.
    let window = &app.app().windows()[0];
    DomHandle::from_rc(window.dom().clone()).detach(node);
    window.request_frame();
    app.settle(8);
    assert!(app.app().windows()[0].image_resources().resources.iter().all(|resource| {
        resource.state == ImageState::Canceled && resource.attached_variant.is_none() && !resource.visible
    }));
    app.shut_down();
}

const LIVE_SOURCE: &str = "assets/live-policy-image.png";

fn live_window(bytes: Arc<[u8]>) -> Harness<Runtime> {
    let handler = App::new().with_size(400.0, 300.0)
        .with_stylesheet("image { display:block; width:64px; height:16px }")
        .with_image_policy(ImagePolicy::package_assets([(LIVE_SOURCE.into(), bytes)]))
        .with_renderer(Box::new(|_, target| {
            let mut renderer = zgui_testkit_scene::CaptureRenderer::new();
            renderer.configure(target);
            Ok(Box::new(renderer))
        }))
        .into_handler(|cx: &mut BuildCx<'_>| {
            Box::new(zgui_elements::image().class("picture")
                .src(Some(String::from(LIVE_SOURCE))).into_view().build(cx))
        }).unwrap();
    Harness::new(handler)
}

/// Ordinary reactive/native wakes must suffice: this never calls the strict publication barrier.
fn await_live_image(app: &mut Harness<Runtime>) -> crate::ImageResource {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        app.settle(8);
        let resources = app.app().windows()[0].image_resources();
        assert!(resources.failures.is_empty(), "{resources:?}");
        assert!(resources.graphics_failure.is_none(), "{resources:?}");
        if let Some(image) = resources.resources.iter().find(|image| {
            image.source == LIVE_SOURCE && image.state != ImageState::Canceled
        }) {
            assert_ne!(image.state, ImageState::Failed, "{resources:?}");
            if image.state == ImageState::Ready {
                assert!(image.uploaded);
                return image.clone();
            }
        }
        assert!(Instant::now() < deadline, "live package image never rendered: {resources:?}");
        std::thread::yield_now();
    }
}

fn replace_live_owner(app: &mut Harness<Runtime>, bytes: Arc<[u8]>) {
    let window = &mut app.app_mut().windows_mut()[0];
    let dom = DomHandle::from_rc(window.dom().clone());
    let old = window.nodes_with_class("picture")[0];
    let parent = dom.parent(old).unwrap();
    dom.detach(old);
    window.replace_image_policy(ImagePolicy::package_assets([(LIVE_SOURCE.into(), bytes)]));
    assert!(window.image_resources().resources.iter().all(|image| image.state == ImageState::Canceled));
    assert_eq!(window.images.held_bytes(), 0, "the retired package cannot remain in the loader cache");
    let image = dom.create_element(ElementName::new("image"));
    dom.set_classes(image, &[ClassName::new("picture")]);
    dom.set_attribute(image, AttrName::new("src"), Some(LIVE_SOURCE));
    dom.insert(parent, image, None);
}

fn blue_png() -> Arc<[u8]> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 2, 2);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header().unwrap().write_image_data(&[0, 0, 255, 255].repeat(4)).unwrap();
    }
    bytes.into()
}

#[test]
fn live_package_images_publish_asynchronously_and_same_path_reload_uses_new_bytes() {
    let red: Arc<[u8]> = Arc::from(include_bytes!("../../tests/fixtures/red2x2.png").as_slice());
    let mut app = live_window(Arc::clone(&red));
    let first = await_live_image(&mut app);
    let first_texels = app.app().windows()[0].images.decoded_texels_for_tests(LIVE_SOURCE).unwrap();
    assert_eq!(&first_texels[..4], &[255, 0, 0, 255]);

    replace_live_owner(&mut app, blue_png());
    let second = await_live_image(&mut app);
    assert_ne!(first.id, second.id);
    assert!(second.generation > first.generation);
    assert_eq!(second.source, first.source, "the package path deliberately stays identical");
    let second_texels = app.app().windows()[0].images.decoded_texels_for_tests(LIVE_SOURCE).unwrap();
    assert_eq!(&second_texels[..4], &[0, 0, 255, 255], "real native decoding must use the new package generation");
    assert_eq!(&first_texels[..4], &[255, 0, 0, 255], "previous immutable readers retain their own bytes");

    // Hold an actual old-generation re-decode across the next owner replacement. Canceling
    // publication must not wait for its blocking worker, nor let its same-path bytes attach.
    let release_old = app.app().windows()[0].images.hold_next_for_tests();
    app.app_mut().windows_mut()[0].forget_caches();
    app.settle(8);
    replace_live_owner(&mut app, red);
    let third = await_live_image(&mut app);
    assert!(third.generation > second.generation);
    release_old.send(()).unwrap();
    app.settle(8);
    let third_texels = app.app().windows()[0].images.decoded_texels_for_tests(LIVE_SOURCE).unwrap();
    assert_eq!(&third_texels[..4], &[255, 0, 0, 255]);
    app.shut_down();
}
