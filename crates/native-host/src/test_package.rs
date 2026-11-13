use crate::package::Package;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, path::PathBuf, rc::Rc, sync::atomic::{AtomicU64, Ordering}};

static NEXT: AtomicU64 = AtomicU64::new(0);
pub(crate) struct Fixture(PathBuf);
impl Fixture {
    pub(crate) fn new(source: &str, css: &str) -> Self {
        let directory = std::env::temp_dir().join(format!("zgui-lifecycle-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        fs::create_dir(&directory).unwrap();
        let files = BTreeMap::from([
            ("app.mjs", source.as_bytes()),
            ("app.mjs.map", br#"{"version":3,"sources":[],"names":[],"mappings":""}"#.as_slice()),
            ("style.css", css.as_bytes()),
            ("test.txt", b"completed".as_slice()),
        ]);
        let mut digest = Sha256::new();
        for (path, bytes) in files {
            fs::write(directory.join(path), bytes).unwrap();
            digest.update(path.as_bytes()); digest.update([0]); digest.update(bytes); digest.update([0]);
        }
        let manifest = serde_json::json!({
            "schemaVersion": 1, "id": "lifecycle-tests", "hostApiVersion": 1,
            "entry": "app.mjs", "sourceMap": "app.mjs.map", "styles": ["style.css"], "assets": ["test.txt"],
            "buildId": format!("sha256:{:x}", digest.finalize()),
        });
        fs::write(directory.join("manifest.json"), serde_json::to_vec(&manifest).unwrap()).unwrap();
        Self(directory)
    }
    pub(crate) fn package(&self) -> Rc<Package> { Rc::new(Package::load(&self.0).unwrap()) }
}
impl Drop for Fixture { fn drop(&mut self) { fs::remove_dir_all(&self.0).unwrap(); } }
