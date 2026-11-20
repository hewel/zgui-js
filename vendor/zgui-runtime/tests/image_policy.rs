//! Strict source authority and readiness through real native layout, codecs and attachment.
mod support;

use std::{sync::Arc, time::{Duration, Instant}};
use zgui_platform_headless::Harness;
use zgui_runtime::{App, ImageBlobs, ImagePolicy, ImageState, Runtime};
use zgui_view::{AttrName, BuildCx, DomHandle, IntoView, View};

const RED: &[u8] = include_bytes!("fixtures/red2x2.png");
const CSS: &str = "root { display:block } image { display:block; width:64px; height:16px }";

fn window(src: &str, css: &str, assets: Vec<(String, Arc<[u8]>)>) -> Harness<Runtime> {
    window_with_policy(src, css, ImagePolicy::package_only(assets))
}

fn window_with_policy(src: &str, css: &str, policy: ImagePolicy) -> Harness<Runtime> {
    let src = src.to_owned();
    let handler = App::new().with_size(400.0, 300.0).with_stylesheet(css)
        .with_renderer(Box::new(support::capture))
        .with_image_policy(policy)
        .into_handler(move |cx: &mut BuildCx<'_>| {
            Box::new(zgui_elements::image().class("picture").src(Some(src.clone())).into_view().build(cx))
        }).unwrap();
    Harness::new(handler)
}

fn set_src(app: &mut Harness<Runtime>, src: Option<&str>) {
    let window = &app.app().windows()[0];
    let node = window.nodes_with_class("picture")[0];
    DomHandle::from_rc(window.dom().clone()).set_attribute(node, AttrName::new("src"), src);
    // Direct fixture DOM writes are outside a native host turn: the fixture owns the redraw.
    window.request_frame();
}

/// Waiting touches only native worker channels. No app time, native frame or RAF can run here.
fn barrier(app: &mut Harness<Runtime>) {
    let now = app.now();
    let frames = app.frames_requested();
    let deadline = Instant::now() + Duration::from_secs(30);
    while !app.app_mut().windows_mut()[0].poll_image_barrier() {
        assert!(Instant::now() < deadline, "native image workers stalled");
        std::thread::yield_now();
    }
    assert_eq!(app.now(), now);
    assert_eq!(app.frames_requested(), frames);
}

fn publish(app: &mut Harness<Runtime>) {
    for _ in 0..8 {
        barrier(app);
        app.settle(8);
        if app.app().windows()[0].image_resources().resources.iter().all(|resource| {
            !resource.visible || matches!(resource.state, ImageState::Ready | ImageState::Failed | ImageState::Canceled)
        }) { return; }
    }
    panic!("native resources never settled: {:?}", app.app().windows()[0].image_resources());
}

#[test]
fn verified_bytes_remain_pending_until_explicit_publication_and_real_attachment() {
    let mut app = window("assets/red.png", CSS, vec![("assets/red.png".into(), Arc::from(RED))]);
    app.settle(8);
    let first = app.app().windows()[0].image_resources();
    assert_eq!(first.resources.len(), 1);
    assert!(first.resources[0].visible);
    assert_eq!(first.resources[0].state, ImageState::Loading);
    assert_eq!(first.resources[0].attached_variant, None);
    // Worker arrival cannot publish, even if an unrelated native frame runs first.
    app.app_mut().windows_mut()[0].request_frame();
    app.settle(8);
    assert_eq!(app.app().windows()[0].image_resources().resources[0].state, ImageState::Loading);
    publish(&mut app);
    let ready = app.app().windows()[0].image_resources();
    assert_eq!(ready.resources[0].state, ImageState::Ready);
    assert!(ready.resources[0].attached_variant >= ready.resources[0].requested_variant);
    assert!(app.app_mut().windows_mut()[0].budget_report().line(zgui_runtime::CacheId::DecodedImages).report.resident > 0);
    app.shut_down();
}

#[test]
fn corrupt_and_undeclared_sources_fail_instead_of_certifying_empty_resources() {
    for src in ["assets/bad.png", "/etc/passwd", "https://example.test/image.png", "zgui-bytes:1", "assets/../bad.png"] {
        let mut app = window(src, CSS, vec![("assets/bad.png".into(), Arc::from(&b"not a raster image"[..]))]);
        app.settle(8);
        publish(&mut app);
        let snapshot = app.app().windows()[0].image_resources();
        assert_eq!(snapshot.resources.len(), 1, "{src}");
        assert_eq!(snapshot.resources[0].state, ImageState::Failed, "{src}");
        assert_eq!(snapshot.resources[0].attached_variant, None);
        assert!(snapshot.resources[0].failure.is_some());
        app.shut_down();
    }
}

#[cfg(target_os = "linux")]
#[test]
fn live_files_are_read_only_beneath_the_granted_directory() {
    use std::fs;
    use std::os::unix::fs::symlink;

    let temp = std::env::temp_dir().join(format!("zgui-image-policy-{}", std::process::id()));
    fs::create_dir(&temp).unwrap();
    struct RemoveTemp(std::path::PathBuf);
    impl Drop for RemoveTemp {
        fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
    }
    let _temp = RemoveTemp(temp.clone());
    let granted = temp.join("grant here");
    let sibling = temp.join("grant here-more");
    fs::create_dir(&granted).unwrap();
    fs::create_dir(&sibling).unwrap();
    fs::write(granted.join("red.png"), RED).unwrap();
    fs::write(sibling.join("red.png"), RED).unwrap();
    fs::write(granted.join("vector.svg"), GREEN_SVG).unwrap();
    symlink("red.png", granted.join("inside.png")).unwrap();
    symlink("../grant here-more/red.png", granted.join("escape.png")).unwrap();
    symlink(&sibling, granted.join("escape-dir")).unwrap();
    symlink(&sibling, temp.join("untrusted-root")).unwrap();
    let oversized = fs::File::create(granted.join("oversized.png")).unwrap();
    oversized.set_len(128 * 1024 * 1024).unwrap();
    let fifo = granted.join("fifo.png");
    rustix::fs::mkfifoat(rustix::fs::CWD, &fifo, rustix::fs::Mode::RUSR).unwrap();

    let uri = |path: &std::path::Path| format!("file://{}", path.to_str().unwrap().replace(' ', "%20"));
    let prefix = granted.to_str().unwrap().to_owned();
    let src = uri(&granted.join("red.png"));
    let mut app = window_with_policy(&src, CSS, ImagePolicy::package_assets([]));
    assert!(await_terminal(&mut app, &src).failures.iter().any(|failure| failure.contains(&src)));
    app.app_mut().windows_mut()[0].replace_image_policy(
        ImagePolicy::package_assets([]).with_file_prefixes(&[
            prefix, temp.join("untrusted-root").to_str().unwrap().to_owned(),
        ]),
    );
    let known = uri(&granted);
    for (src, expected) in [
        (uri(&granted.join("inside.png")), ImageState::Ready),
        (uri(&granted.join("vector.svg")), ImageState::Ready),
        (uri(&sibling.join("red.png")), ImageState::Failed),
        (uri(&granted.join("escape.png")), ImageState::Failed),
        (uri(&granted.join("escape-dir/red.png")), ImageState::Failed),
        (uri(&temp.join("untrusted-root/red.png")), ImageState::Failed),
        (format!("{known}/../grant%20here-more/red.png"), ImageState::Failed),
        (format!("{known}/%2E%2E/grant%20here-more/red.png"), ImageState::Failed),
        (format!("{known}%2Finside.png"), ImageState::Failed),
        (format!("{known}//inside.png"), ImageState::Failed),
        (format!("{known}/inside.png?raw=1"), ImageState::Failed),
        (format!("{known}/inside.png#fragment"), ImageState::Failed),
        (uri(&granted.join("oversized.png")), ImageState::Failed),
        (uri(&granted.join("fifo.png")), ImageState::Failed),
        (uri(&granted.join("escape-dir")), ImageState::Failed),
    ] {
        set_src(&mut app, Some(&src));
        let terminal = await_terminal(&mut app, &src);
        assert_eq!(terminal.resources.last().unwrap().state, expected, "{src}: {terminal:?}");
    }
    app.shut_down();
}

#[cfg(target_os = "linux")]
fn await_terminal(app: &mut Harness<Runtime>, src: &str) -> zgui_runtime::ImageResources {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        app.settle(8);
        let resources = app.app().windows()[0].image_resources();
        if resources.resources.last().is_some_and(|image| {
            image.source == src && matches!(image.state, ImageState::Ready | ImageState::Failed)
        }) {
            return resources;
        }
        assert!(Instant::now() < deadline, "{src} never settled: {resources:?}");
        std::thread::yield_now();
    }
}

const GREEN_SVG: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="16">
  <rect width="64" height="16" fill="green"/>
</svg>"#;

#[test]
fn package_svg_keeps_the_raster_only_decoder() {
    let mut app = window("assets/vector.svg", CSS, vec![("assets/vector.svg".into(), Arc::from(GREEN_SVG))]);
    app.settle(8);
    publish(&mut app);
    let snapshot = app.app().windows()[0].image_resources();
    assert_eq!(snapshot.resources[0].state, ImageState::Failed, "{snapshot:?}");
    assert!(snapshot.resources[0].attached_variant.is_none());
    app.shut_down();
}

#[test]
fn fixture_files_require_both_exact_mapping_and_prefix_without_ambient_reads() {
    let src = "file:///fixture/granted/does-not-exist.svg";
    let sibling = "file:///fixture/granted-other/picture.svg";
    let alternate = "file:///fixture/granted/does-not-exist.png";
    let mut app = window_with_policy(src, CSS, ImagePolicy::package_only([]).with_fixture_files(
        &["/fixture/granted".into()],
        [(src.into(), Arc::from(GREEN_SVG)), (sibling.into(), Arc::from(GREEN_SVG))],
    ));
    app.settle(8);
    publish(&mut app);
    let ready = app.app().windows()[0].image_resources();
    assert_eq!(ready.resources[0].state, ImageState::Ready, "{ready:?}");
    for denied in [sibling, alternate] {
        set_src(&mut app, Some(denied));
        app.settle(8);
        publish(&mut app);
        let snapshot = app.app().windows()[0].image_resources();
        assert_eq!(snapshot.resources.last().unwrap().state, ImageState::Failed, "{snapshot:?}");
        assert!(snapshot.failures.iter().any(|failure| failure.contains(denied)));
    }
    app.shut_down();
}

#[test]
fn svg_cannot_resolve_external_or_nested_images() {
    let sources = [
        "file:///fixture/granted/external.png",
        "relative.png",
        "https://example.test/external.png",
        "data:image/svg+xml,%3Csvg%20xmlns='http://www.w3.org/2000/svg'%20width='1'%20height='1'/%3E",
    ];
    let blobs = ImageBlobs::new(10);
    let image_uris: Vec<_> = sources.map(|source| {
        let svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="16">
            <rect width="64" height="16" fill="green"/>
            <image width="64" height="16" href="{source}"/>
        </svg>"#);
        blobs.register(Arc::from(svg.into_bytes())).unwrap()
    }).into_iter().collect();
    let mut app = window_with_policy(&image_uris[0], CSS, ImagePolicy::package_only([]).with_blobs(blobs));
    for (src, external) in image_uris.iter().zip(sources) {
        set_src(&mut app, Some(src));
        app.settle(8);
        publish(&mut app);
        let snapshot = app.app().windows()[0].image_resources();
        let resource = snapshot.resources.last().unwrap();
        assert_eq!(resource.state, ImageState::Failed, "{external}: {snapshot:?}");
        assert!(resource.attached_variant.is_none(), "{external}: {snapshot:?}");
        assert!(resource.failure.as_deref().unwrap().contains("SVG image references"));
    }
    app.shut_down();
}

#[test]
fn svg_resolves_embedded_raster_data_uri() {
    const TABLE: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::with_capacity(RED.len().div_ceil(3) * 4);
    for chunk in RED.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = chunk.get(1).copied().map_or(0, u32::from);
        let b2 = chunk.get(2).copied().map_or(0, u32::from);
        let triple = (b0 << 16) | (b1 << 8) | b2;
        encoded.push(TABLE[((triple >> 18) & 0x3f) as usize] as char);
        encoded.push(TABLE[((triple >> 12) & 0x3f) as usize] as char);
        encoded.push(if chunk.len() > 1 { TABLE[((triple >> 6) & 0x3f) as usize] as char } else { '=' });
        encoded.push(if chunk.len() > 2 { TABLE[(triple & 0x3f) as usize] as char } else { '=' });
    }
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="64" height="16">
            <image width="64" height="16" xlink:href="data:image/png;base64,{encoded}"/>
        </svg>"#
    );
    let blobs = ImageBlobs::new(10);
    let src = blobs.register(Arc::from(svg.into_bytes())).unwrap();
    let mut app = window_with_policy(&src, CSS, ImagePolicy::package_only([]).with_blobs(blobs));
    app.settle(8);
    publish(&mut app);
    let snapshot = app.app().windows()[0].image_resources();
    let resource = snapshot.resources.last().unwrap();
    assert_eq!(resource.state, ImageState::Ready, "{snapshot:?}");
    assert!(resource.attached_variant.is_some(), "{snapshot:?}");
    assert!(resource.failure.is_none(), "{snapshot:?}");
    app.shut_down();
}

#[test]
fn a_queued_blob_completion_cannot_attach_after_release() {
    let blobs = ImageBlobs::new(9);
    let src = blobs.register(Arc::from(RED)).unwrap();
    let mut app = window_with_policy(
        &src,
        CSS,
        ImagePolicy::package_only([]).with_blobs(blobs.clone()),
    );
    app.settle(8);
    assert_eq!(app.app().windows()[0].image_resources().resources[0].state, ImageState::Loading);
    barrier(&mut app);
    assert!(blobs.release(&src));
    app.app().windows()[0].request_frame();
    app.settle(8);
    publish(&mut app);
    let snapshot = app.app().windows()[0].image_resources();
    let resource = &snapshot.resources[0];
    assert_eq!(resource.state, ImageState::Failed, "{snapshot:?}");
    assert_eq!(resource.attached_variant, None);
    assert!(!resource.uploaded);
    app.shut_down();
}

#[test]
fn a_blob_released_after_a_same_turn_clear_or_overwrite_does_not_fail_policy() {
    const RASTER: &str = "assets/red.png";
    let blobs = ImageBlobs::new(9);
    let mut app = window_with_policy(
        RASTER,
        CSS,
        ImagePolicy::package_only([(RASTER.into(), Arc::from(RED))]).with_blobs(blobs.clone()),
    );
    app.settle(8);
    publish(&mut app);
    for next in [None, Some(RASTER)] {
        let src = blobs.register(Arc::from(RED)).unwrap();
        set_src(&mut app, Some(&src));
        set_src(&mut app, next);
        assert!(blobs.release(&src));
        app.settle(8);
        publish(&mut app);
        let snapshot = app.app().windows()[0].image_resources();
        assert!(snapshot.failures.is_empty(), "{snapshot:?}");
        assert_eq!(
            snapshot.resources.iter().find(|image| image.state == ImageState::Ready).map(|image| image.source.as_str()),
            next,
        );
    }
    app.shut_down();
}

#[test]
fn releasing_a_blob_and_replacing_a_policy_cannot_reuse_old_authority() {
    let old = ImageBlobs::new(4);
    let old_src = old.register(Arc::from(RED)).unwrap();
    let abandoned = old.register(Arc::from(RED)).unwrap();
    let new = ImageBlobs::new(5);
    let new_src = new.register(Arc::from(GREEN_SVG)).unwrap();
    assert_ne!(old_src, new_src);
    let mut app = window_with_policy(&old_src, CSS, ImagePolicy::package_only([]).with_blobs(old.clone()));
    app.settle(8);
    publish(&mut app);
    assert_eq!(app.app().windows()[0].image_resources().resources[0].state, ImageState::Ready);

    assert!(old.release(&old_src));
    assert!(!old.release(&old_src));
    assert_eq!(old.len(), 1);
    app.app().windows()[0].request_frame();
    app.settle(8);
    let released = app.app().windows()[0].image_resources();
    assert_eq!(released.resources[0].state, ImageState::Failed);
    assert_eq!(released.resources[0].attached_variant, None);
    assert!(!released.resources[0].uploaded);
    set_src(&mut app, None);
    app.settle(8);
    set_src(&mut app, Some(&old_src));
    app.settle(8);
    publish(&mut app);
    assert_eq!(app.app().windows()[0].image_resources().resources.last().unwrap().state, ImageState::Failed);

    app.app_mut().windows_mut()[0].replace_image_policy(ImagePolicy::package_only([]).with_blobs(new.clone()));
    assert!(old.is_empty());
    old.release(&new_src);
    new.release(&abandoned);
    assert_eq!(new.len(), 1);
    set_src(&mut app, Some(&abandoned));
    app.settle(8);
    publish(&mut app);
    assert_eq!(app.app().windows()[0].image_resources().resources.last().unwrap().state, ImageState::Failed);
    set_src(&mut app, Some(&new_src));
    app.settle(8);
    publish(&mut app);
    assert_eq!(app.app().windows()[0].image_resources().resources.last().unwrap().state, ImageState::Ready);
    app.shut_down();
}

#[test]
fn replaced_and_cleared_sources_retire_ids_and_cannot_attach_stale_work() {
    let assets = vec![("first.png".into(), Arc::from(RED)), ("second.png".into(), Arc::from(RED))];
    let mut app = window("first.png", CSS, assets);
    app.settle(8);
    let first_id = app.app().windows()[0].image_resources().resources[0].id.clone();
    set_src(&mut app, Some("second.png"));
    app.settle(8);
    set_src(&mut app, Some("first.png"));
    app.settle(8);
    publish(&mut app);
    let snapshot = app.app().windows()[0].image_resources();
    let active = snapshot.resources.iter().find(|resource| resource.state == ImageState::Ready).unwrap();
    assert_eq!(active.source, "first.png");
    assert_ne!(active.id, first_id);
    assert_eq!(snapshot.resources.iter().filter(|resource| resource.state == ImageState::Canceled).count(), 2);
    set_src(&mut app, None);
    app.settle(8);
    assert!(app.app().windows()[0].image_resources().resources.iter().all(|resource| resource.state == ImageState::Canceled && resource.attached_variant.is_none()));
    app.shut_down();
}

fn wide_png() -> Arc<[u8]> { raster(512) }

fn raster(width: u32) -> Arc<[u8]> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, width, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header().unwrap().write_image_data(&[255, 0, 0, 255].repeat(width as usize)).unwrap();
    }
    bytes.into()
}

#[test]
fn dpr_growth_does_not_allow_an_attached_thumbnail_to_certify_new_demand() {
    let mut app = window("wide.png", CSS, vec![("wide.png".into(), wide_png())]);
    app.settle(8);
    publish(&mut app);
    let previous = app.app().windows()[0].image_resources().resources[0].clone();
    assert_eq!(previous.state, ImageState::Ready);
    app.deliver_to_first(zgui_platform::SurfaceEvent::ScaleFactorChanged {
        scale_factor: 2.0,
        size: zgui_geom::Size::new(zgui_geom::DevicePx(800.0), zgui_geom::DevicePx(600.0)),
    });
    app.settle(8);
    let pending = app.app().windows()[0].image_resources().resources[0].clone();
    assert_eq!(pending.id, previous.id);
    assert_eq!(pending.state, ImageState::Loading);
    assert!(pending.requested_variant > previous.requested_variant);
    assert!(pending.attached_variant < pending.requested_variant);
    publish(&mut app);
    let ready = app.app().windows()[0].image_resources().resources[0].clone();
    assert_eq!(ready.state, ImageState::Ready);
    assert!(ready.attached_variant >= ready.requested_variant);
    app.shut_down();
}

#[test]
fn package_and_inline_css_resource_references_fail_closed_including_escapes() {
    for css in [
        r#"image { background: url(https://example.test/a.png) }"#,
        r#"image { background: u\72l('ambient.png') }"#,
        r#"@\69mport 'https://example.test/style.css';"#,
        r#"image { background: image-set('ambient.png' 1x) }"#,
    ] {
        let mut app = window("red.png", css, vec![("red.png".into(), Arc::from(RED))]);
        app.settle(8);
        assert!(!app.app().windows()[0].image_resources().failures.is_empty(), "{css}");
        app.shut_down();
    }
    let mut app = window("red.png", CSS, vec![("red.png".into(), Arc::from(RED))]);
    app.settle(8);
    let window = &app.app().windows()[0];
    let node = window.nodes_with_class("picture")[0];
    DomHandle::from_rc(window.dom().clone()).set_style_text(node, Some(r#"background: url('ambient.png')"#));
    window.request_frame();
    app.settle(8);
    assert!(!app.app().windows()[0].image_resources().failures.is_empty());
    app.shut_down();
}

#[test]
fn real_atlas_allocation_refusal_cannot_certify_decoded_images_as_ready() {
    // Every image is on-screen and decoded through the real loader. Mips require standalone
    // textures, deliberately exhausting the pinned native atlas's default 16-texture pool.
    let bytes = raster(1024);
    let assets = (0..17).map(|index| (format!("image-{index}.png"), Arc::clone(&bytes)));
    let handler = App::new().with_size(1100.0, 100.0)
        .with_stylesheet("column { display:flex; flex-direction:column; width:1024px; height:17px } image { display:block; width:1024px; height:1px; flex-shrink:0 }")
        .with_renderer(Box::new(support::capture))
        .with_image_policy(ImagePolicy::package_only(assets))
        .into_handler(|cx: &mut BuildCx<'_>| {
            let mut pictures = zgui_elements::column();
            for index in 0..17 {
                pictures = pictures.child(zgui_elements::image().src(Some(format!("image-{index}.png"))));
            }
            Box::new(pictures.into_view().build(cx))
        }).unwrap();
    let mut app = Harness::new(handler);
    app.settle(8);
    let loading = app.app().windows()[0].image_resources();
    assert_eq!(loading.resources.len(), 17);
    assert!(loading.resources.iter().all(|image| image.state == ImageState::Loading && image.failure.is_none()));
    let refused_before = zgui_profile::counter::get(zgui_profile::Counter::ImageInsertFailed);
    publish(&mut app);
    let snapshot = app.app().windows()[0].image_resources();
    assert!(zgui_profile::counter::get(zgui_profile::Counter::ImageInsertFailed) > refused_before,
        "the native atlas must actually refuse an allocation, not a fabricated status");
    assert!(snapshot.graphics_failure.is_none(), "capacity refusal is an asset failure, not a device error");
    assert!(snapshot.resources.iter().any(|image| image.state == ImageState::Ready && image.uploaded));
    assert!(snapshot.resources.iter().any(|image| {
        image.visible && image.state == ImageState::Failed && !image.uploaded
            && image.failure.as_deref().is_some_and(|message| message.contains("atlas"))
    }), "an omitted image must never be certified ready: {snapshot:?}");
    app.shut_down();
}

#[test]
fn hidden_sources_do_not_block_visible_readiness_and_upload_when_shown() {
    for (css, property, value) in [
        ("image { display:block; width:64px; height:16px; margin-top:600px }", "margin-top", "0px"),
        ("root { display:block; width:32px; height:16px; overflow:hidden } image { display:block; width:64px; height:16px; margin-left:64px }", "margin-left", "0px"),
        ("image { display:block; width:64px; height:16px; opacity:0 }", "opacity", "1"),
    ] {
        let assets = vec![("first.png".into(), Arc::from(RED)), ("second.png".into(), Arc::from(RED))];
        let mut app = window("first.png", css, assets);
        app.settle(8);
        publish(&mut app);
        // Replacing an already-laid-out, hidden image exercises incremental viewport damage;
        // an initial full-damage frame can otherwise allocate content outside the surface.
        set_src(&mut app, Some("second.png"));
        app.settle(8);
        publish(&mut app);
        let hidden = app.app().windows()[0].image_resources();
        let current = hidden.resources.iter().find(|image| image.source == "second.png").unwrap();
        assert!(!current.visible, "{css}: {hidden:?}");
        assert!(current.failure.is_none());
        let id = current.id.clone();
        let window = &app.app().windows()[0];
        let node = window.nodes_with_class("picture")[0];
        DomHandle::from_rc(window.dom().clone()).set_style_property(node, property, Some(value));
        window.request_frame();
        app.settle(8);
        publish(&mut app);
        let shown = app.app().windows()[0].image_resources();
        let current = shown.resources.iter().find(|image| image.id == id).unwrap();
        assert!(current.visible, "{css}: {shown:?}");
        assert!(current.uploaded, "visible readiness still requires native texture upload");
        assert_eq!(current.state, ImageState::Ready);
        assert!(current.failure.is_none());
        app.shut_down();
    }
}

#[test]
fn failed_image_state_drives_fallback_and_resets_with_its_source() {
    const FALLBACK_CSS: &str = "root { display:block } image { display:block; width:64px; height:16px } .picture:state(zgui-image-broken) { display:none }";
    let broken_name = zgui_interned::Ident::new("zgui-image-broken");
    let mut app = window("assets/bad.png", FALLBACK_CSS, vec![
        ("assets/bad.png".into(), Arc::from(&b"not a raster image"[..])),
        ("assets/good.png".into(), Arc::from(RED)),
    ]);
    app.settle(8);
    let (node, document) = {
        let window = &app.app().windows()[0];
        (window.nodes_with_class("picture")[0], window.document().clone())
    };
    let index = {
        let document = document.borrow();
        (0..document.store().slot_count())
            .map(|slot| zgui_dom::NodeIndex::new(slot as u32))
            .find(|&index| document.store().classes_of(index).iter().any(|class| &**class == "picture"))
            .unwrap()
    };
    document.borrow().edit(&zgui_dom::EverythingMatters, |edit| {
        edit.set_state(index, zgui_vocab::UiState::HOVER, true);
    }).unwrap();
    app.app_mut().windows_mut()[0].request_frame();

    let ui_state = |app: &Harness<Runtime>| {
        app.app().windows()[0].dom().ui_state(node)
    };
    let broken_state = |app: &Harness<Runtime>| {
        app.app().windows()[0].dom().has_custom_state(node, broken_name)
    };

    let loading = app.app().windows()[0].image_resources();
    assert_eq!(loading.resources.len(), 1);
    assert!(loading.resources[0].visible);
    assert_eq!(loading.resources[0].state, ImageState::Loading);
    assert!(!ui_state(&app).contains(zgui_vocab::UiState::BROKEN));
    assert!(!broken_state(&app));

    publish(&mut app);
    let failed = app.app().windows()[0].image_resources();
    let bad = failed.resources.iter().rev().find(|image| image.source == "assets/bad.png").unwrap();
    assert_eq!(bad.state, ImageState::Failed);
    assert!(!bad.visible, "the :state rule hides the failed picture: {failed:?}");
    assert!(ui_state(&app).contains(zgui_vocab::UiState::BROKEN));
    assert!(broken_state(&app));
    assert!(ui_state(&app).contains(zgui_vocab::UiState::HOVER));

    set_src(&mut app, Some("assets/good.png"));
    app.settle(8);
    assert!(!ui_state(&app).contains(zgui_vocab::UiState::BROKEN));
    assert!(!broken_state(&app));
    assert!(ui_state(&app).contains(zgui_vocab::UiState::HOVER));
    publish(&mut app);
    let ready = app.app().windows()[0].image_resources();
    let good = ready.resources.iter().rev().find(|image| image.source == "assets/good.png").unwrap();
    assert_eq!(good.state, ImageState::Ready);
    assert!(good.visible);

    set_src(&mut app, Some("assets/bad.png"));
    publish(&mut app);
    let failed = app.app().windows()[0].image_resources();
    let bad = failed.resources.iter().rev().find(|image| image.source == "assets/bad.png").unwrap();
    assert_eq!(bad.state, ImageState::Failed);
    assert!(!bad.visible);
    assert!(ui_state(&app).contains(zgui_vocab::UiState::BROKEN));
    assert!(broken_state(&app));

    set_src(&mut app, None);
    app.settle(8);
    assert!(!ui_state(&app).contains(zgui_vocab::UiState::BROKEN));
    assert!(!broken_state(&app));

    set_src(&mut app, Some("assets/bad.png"));
    app.settle(8);
    let pending = app.app().windows()[0].image_resources();
    let bad = pending.resources.iter().rev().find(|image| image.source == "assets/bad.png").unwrap();
    assert_eq!(bad.state, ImageState::Loading);
    set_src(&mut app, Some("assets/good.png"));
    app.settle(8);
    publish(&mut app);
    let ready = app.app().windows()[0].image_resources();
    let good = ready.resources.iter().rev().find(|image| image.source == "assets/good.png").unwrap();
    assert_eq!(good.state, ImageState::Ready);
    assert!(good.visible);
    assert!(!ui_state(&app).contains(zgui_vocab::UiState::BROKEN));
    assert!(!broken_state(&app));
    assert_eq!(app.app().windows()[0].nodes_with_class("picture")[0], node);

    set_src(&mut app, Some("assets/bad.png"));
    app.settle(8);
    publish(&mut app);
    assert!(ui_state(&app).contains(zgui_vocab::UiState::BROKEN));
    assert!(broken_state(&app));
    app.app_mut().windows_mut()[0].replace_image_policy(ImagePolicy::package_only([]));
    app.settle(8);
    assert!(!ui_state(&app).contains(zgui_vocab::UiState::BROKEN));
    assert!(!broken_state(&app));
    assert!(ui_state(&app).contains(zgui_vocab::UiState::HOVER));
    assert!(app.app().windows()[0].image_resources().resources.iter().all(|image| image.state == ImageState::Canceled));
    app.shut_down();
}
