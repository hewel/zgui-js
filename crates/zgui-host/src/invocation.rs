//! Application-visible launch context, independent of the window system.
use std::{ffi::OsString, path::Path};

pub(crate) const MAX_ARGS: usize = 64;
pub(crate) const MAX_ARG_BYTES: usize = 64 * 1024;

crate::schema::object_struct! {
    #[serde(deny_unknown_fields)]
    pub(crate) struct ActivationInfo {
        pub args: Vec<String>,
        pub cwd: Option<String>,
    }
}

impl ActivationInfo {
    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        if self.args.len() > MAX_ARGS {
            return Err("activation accepts at most 64 application arguments");
        }
        if self.args.iter().map(String::len).sum::<usize>() > MAX_ARG_BYTES {
            return Err("activation accepts at most 64 KiB of application arguments (UTF-8)");
        }
        if self.cwd.as_ref().is_some_and(|cwd| !Path::new(cwd).is_absolute() || cwd.contains('\0')) {
            return Err("activation cwd must be an absolute path or null");
        }
        Ok(())
    }
}

pub(crate) struct Activation {
    pub info: ActivationInfo,
    pub token: Option<String>,
}

impl ActivationInfo {
    pub(crate) fn launch(args: impl Iterator<Item = OsString>) -> Result<Self, &'static str> {
        let mut values = Vec::new();
        let mut bytes = 0;
        for arg in args {
            if values.len() == MAX_ARGS { return Err("activation accepts at most 64 application arguments"); }
            let arg = arg.into_string().map_err(|_| "application arguments must be valid UTF-8")?;
            bytes += arg.len();
            if bytes > MAX_ARG_BYTES { return Err("activation accepts at most 64 KiB of application arguments (UTF-8)"); }
            values.push(arg);
        }
        let cwd = std::env::current_dir().and_then(|path| path.canonicalize()).ok()
            .filter(|path| path.is_absolute()).and_then(|path| path.into_os_string().into_string().ok());
        Ok(Self { args: values, cwd })
    }
}

pub(crate) fn valid_token(token: &str) -> bool {
    !token.is_empty() && !token.bytes().any(|byte| byte.is_ascii_control())
}
