use crate::package::Package;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, path::PathBuf, rc::Rc, sync::atomic::{AtomicU64, Ordering}};

static NEXT: AtomicU64 = AtomicU64::new(0);
pub(crate) struct Fixture(PathBuf);
impl Fixture {
    pub(crate) fn new(source: &str, css: &str) -> Self {
        Self::with_styles(source, &[("style.css", css)])
    }
    pub(crate) fn with_styles(source: &str, styles: &[(&str, &str)]) -> Self {
        let directory = std::env::temp_dir().join(format!("zgui-lifecycle-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        fs::create_dir(&directory).unwrap();
        let style_system = serde_json::json!({"name":"stylex","profile":"zgui-stylex-v1",
            "configDigest":format!("sha256:{:x}", Sha256::digest(b"test")),"nativeProfileDigest":crate::package::native_profile_digest()});
        let metadata = serde_json::to_vec(&serde_json::json!({"styleSystem":style_system})).unwrap();
        let mut files = BTreeMap::from([
            ("app.mjs", source.as_bytes()),
            ("app.mjs.map", br#"{"version":3,"sources":[],"names":[],"mappings":""}"#.as_slice()),
            ("test.txt", b"completed".as_slice()),
            ("style-origins.json", metadata.as_slice()),
            ("build-info.json", metadata.as_slice()),
        ]);
        files.extend(styles.iter().map(|(path, css)| (*path, css.as_bytes())));
        let mut digests = BTreeMap::new();
        for (path, bytes) in files {
            fs::write(directory.join(path), bytes).unwrap();
            digests.insert(path, format!("sha256:{:x}", Sha256::digest(bytes)));
        }
        let mut manifest = serde_json::json!({
            "schemaVersion": 2, "id": "lifecycle-tests", "hostApiVersion": 1,
            "entry": "app.mjs", "sourceMap": "app.mjs.map", "styles": styles.iter().map(|(path, _)| path).collect::<Vec<_>>(), "assets": ["test.txt"],
            "fonts": [], "artifactDigests": digests,
            "styleSystem": style_system
        });
        manifest["buildId"] = serde_json::json!(format!("sha256:{:x}", Sha256::digest(serde_json::to_vec(&manifest).unwrap())));
        fs::write(directory.join("manifest.json"), serde_json::to_vec(&manifest).unwrap()).unwrap();
        Self(directory)
    }
    pub(crate) fn package(&self) -> Rc<Package> { Rc::new(Package::load(&self.0).unwrap()) }
    /// Seal capability declarations into the test package identity.
    pub(crate) fn with_capabilities(self, capabilities: crate::package::Capabilities) -> Self {
        let path = self.0.join("manifest.json");
        let mut manifest: serde_json::Value = serde_json::from_slice(&fs::read(&path).expect("read test manifest"))
            .expect("decode test manifest");
        manifest["capabilities"] = serde_json::to_value(capabilities).expect("encode test capabilities");
        manifest.as_object_mut().expect("test manifest object").remove("buildId");
        manifest["buildId"] = serde_json::json!(format!("sha256:{:x}", Sha256::digest(serde_json::to_vec(&manifest).expect("encode manifest identity"))));
        fs::write(path, serde_json::to_vec(&manifest).expect("encode sealed manifest")).expect("write sealed test manifest");
        self
    }
    pub(crate) fn with_font(self, family: &str, bytes: &[u8]) -> Self {
        fs::write(self.0.join("font.ttf"), bytes).unwrap();
        let mut manifest: serde_json::Value = serde_json::from_slice(&fs::read(self.0.join("manifest.json")).unwrap()).unwrap();
        manifest["assets"].as_array_mut().unwrap().push(serde_json::json!("font.ttf"));
        manifest["fonts"] = serde_json::json!([{"family":family,"path":"font.ttf"}]);
        manifest["artifactDigests"]["font.ttf"] = serde_json::json!(format!("sha256:{:x}", Sha256::digest(bytes)));
        manifest.as_object_mut().unwrap().remove("buildId");
        manifest["buildId"] = serde_json::json!(format!("sha256:{:x}", Sha256::digest(serde_json::to_vec(&manifest).unwrap())));
        fs::write(self.0.join("manifest.json"), serde_json::to_vec(&manifest).unwrap()).unwrap();
        self
    }
}
impl Drop for Fixture { fn drop(&mut self) { fs::remove_dir_all(&self.0).unwrap(); } }
