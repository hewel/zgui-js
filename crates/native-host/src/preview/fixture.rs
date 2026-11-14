//! Closed fixture preflight; providers share the production service boundary.
use std::{
    fs::{self, File},
    io::Read,
    path::Path,
};
pub(crate) use crate::work::fixture::FixtureDefinition;
use super::{Error, output::digest};

const MAX_INPUT_BYTES: u64 = 1024 * 1024;

#[derive(Debug)]
pub struct LoadedFixture {
    pub id: String,
    /// Exact input bytes, including whitespace, for the saved run inputs.
    pub original: Vec<u8>,
    pub digest: String,
    pub definition: FixtureDefinition,
}


pub fn load(path: &Path) -> Result<LoadedFixture, Error> {
    let metadata = fs::metadata(path)
        .map_err(|error| invalid(format!("fixture {}: {error}", path.display())))?;
    if !metadata.is_file() || metadata.len() > MAX_INPUT_BYTES {
        return Err(invalid("fixture must be a regular file of at most 1 MiB"));
    }
    let mut original = Vec::with_capacity(metadata.len() as usize);
    File::open(path)
        .and_then(|file| file.take(MAX_INPUT_BYTES + 1).read_to_end(&mut original))
        .map_err(|error| invalid(format!("fixture {}: {error}", path.display())))?;
    if original.len() as u64 > MAX_INPUT_BYTES {
        return Err(invalid("fixture exceeds 1 MiB"));
    }
    let definition = parse(&original)?;
    let id = definition.id.clone();
    let digest = digest(&original);
    Ok(LoadedFixture { id, original, digest, definition })
}

fn parse(bytes: &[u8]) -> Result<FixtureDefinition, Error> {
    let fixture: FixtureDefinition = serde_json::from_slice(bytes)
        .map_err(|error| invalid(format!("invalid fixture: {error}")))?;
    fixture.validate().map_err(invalid)?;
    Ok(fixture)
}

fn invalid(message: impl Into<String>) -> Error { Error::new("E_SCHEMA", 2, message) }

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn empty_providers_and_events_are_explicit_not_defaulted() {
        assert_eq!(parse(br#"{"schemaVersion":1,"id":"empty","services":{},"events":[]}"#).unwrap().id, "empty");
        for bytes in [
            br#"{"schemaVersion":1,"id":"empty","events":[]}"#.as_slice(),
            br#"{"schemaVersion":1,"id":"empty","services":{}}"#.as_slice(),
            br#"{"schemaVersion":1,"id":"empty","services":[],"events":[]}"#.as_slice(),
            br#"{"schemaVersion":1,"id":"empty","services":{},"events":{}}"#.as_slice(),
            br#"{"schemaVersion":1,"id":"empty","services":null,"events":[]}"#.as_slice(),
            br#"{"schemaVersion":1,"id":"empty","services":{},"events":null}"#.as_slice(),
            br#"{"schemaVersion":1,"id":"empty","services":{},"events":[],"live":true}"#.as_slice(),
            br#"{"schemaVersion":2,"id":"empty","services":{},"events":[]}"#.as_slice(),
            br#"{"schemaVersion":"1","id":"empty","services":{},"events":[]}"#.as_slice(),
            br#"{"schemaVersion":1,"id":"empty","id":"other","services":{},"events":[]}"#.as_slice(),
            br#"{"schemaVersion":1,"id":"empty","services":{},"services":{},"events":[]}"#.as_slice(),
            br#"{"schemaVersion":1,"id":" ","services":{},"events":[]}"#.as_slice(),
        ] {
            let error = parse(bytes).unwrap_err();
            assert_eq!((error.code, error.exit_code), ("E_SCHEMA", 2), "{error}");
        }
    }

    #[test]
    fn unknown_providers_and_untyped_events_are_rejected_before_mount() {
        for (services, events) in [
            (json!({"audio":{"volume":0.4}}), json!([])),
            (json!({}), json!([{"atMs":0,"channel":"audio","value":{"muted":true}}])),
            (json!({"assets":{"provider":"live-assets","latencyMs":0,"initialState":{"assets":{}}}}), json!([])),
        ] {
            let bytes = serde_json::to_vec(&json!({"schemaVersion":1,"id":"invalid","services":services,"events":events})).unwrap();
            let error = parse(&bytes).unwrap_err();
            assert_eq!((error.code, error.exit_code), ("E_SCHEMA", 2));
        }
    }

    struct InputFile(std::path::PathBuf);

    impl InputFile {
        fn new(bytes: &[u8]) -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!("zgui-preview-fixture-{}-{}.json", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            use std::io::Write;
            let mut file = fs::OpenOptions::new().write(true).create_new(true).open(&path).unwrap();
            file.write_all(bytes).unwrap();
            Self(path)
        }
    }

    impl Drop for InputFile {
        fn drop(&mut self) { fs::remove_file(&self.0).unwrap(); }
    }

    #[test]
    fn inclusive_input_limit_preserves_original_bytes_and_digest() {
        let mut bytes = b"{\n  \"schemaVersion\": 1, \"id\": \"empty\", \"services\": {}, \"events\": []\n}\n".to_vec();
        bytes.resize(MAX_INPUT_BYTES as usize, b' ');
        let file = InputFile::new(&bytes);
        let loaded = load(&file.0).unwrap();
        assert_eq!(loaded.id, "empty");
        assert_eq!(loaded.original, bytes);
        assert_eq!(loaded.digest, digest(&bytes));
        bytes.push(b' ');
        let too_large = InputFile::new(&bytes);
        let error = load(&too_large.0).unwrap_err();
        assert_eq!((error.code, error.exit_code), ("E_SCHEMA", 2));
        assert_eq!(load(&std::env::temp_dir()).unwrap_err().code, "E_SCHEMA");
    }
}
