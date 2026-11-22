//! A publication observer, not a second runtime: each accepted package gets one
//! ordinary run controller, and the next worker starts only after that owner joins.
use std::{fs::{self, File, OpenOptions}, io::Write, path::{Path, PathBuf}, sync::{Arc, atomic::{AtomicBool, AtomicI32, Ordering}}, thread::{self, JoinHandle}, time::Duration};
use serde_json::{Value, json};
use super::{Error, output::{self, Output}, run};

const STATUS_FILE: &str = ".zgui-build-status.json";
const POLL: Duration = Duration::from_millis(50);
static SIGNAL: AtomicI32 = AtomicI32::new(0);
extern "C" fn interrupt(signal: libc::c_int) { SIGNAL.store(signal, Ordering::Relaxed); }

struct Signals(Vec<(libc::c_int, libc::sigaction)>);
impl Signals {
    fn install() -> Result<Self, Error> {
        SIGNAL.store(0, Ordering::Relaxed);
        let mut guard = Self(Vec::new());
        for signal in [libc::SIGINT, libc::SIGTERM] {
            let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
            let mut old: libc::sigaction = unsafe { std::mem::zeroed() };
            action.sa_sigaction = interrupt as *const () as usize;
            unsafe { libc::sigemptyset(&mut action.sa_mask); }
            if unsafe { libc::sigaction(signal, &action, &mut old) } != 0 { return Err(io(std::io::Error::last_os_error())); }
            guard.0.push((signal, old));
        }
        Ok(guard)
    }
}
impl Drop for Signals {
    fn drop(&mut self) { for (signal, old) in &self.0 { unsafe { libc::sigaction(*signal, old, std::ptr::null_mut()); } } }
}


struct NonblockingOutput(Vec<(libc::c_int, libc::c_int)>);
impl NonblockingOutput {
    fn install()->Result<Self,Error> {
        let mut guard=Self(Vec::new());
        for fd in [libc::STDOUT_FILENO,libc::STDERR_FILENO] {
            let flags=unsafe { libc::fcntl(fd,libc::F_GETFL) };
            if flags<0 || unsafe { libc::fcntl(fd,libc::F_SETFL,flags|libc::O_NONBLOCK) }<0 {
                return Err(io(std::io::Error::last_os_error()));
            }
            guard.0.push((fd,flags));
        }
        Ok(guard)
    }
}
impl Drop for NonblockingOutput {
    fn drop(&mut self) { for (fd,flags) in self.0.iter().rev() { unsafe { libc::fcntl(*fd,libc::F_SETFL,*flags); } } }
}
fn stdout(mut bytes:&[u8])->Result<(),Error> {
    let deadline=std::time::Instant::now()+Duration::from_millis(250);
    while !bytes.is_empty() {
        let count=unsafe { libc::write(libc::STDOUT_FILENO,bytes.as_ptr().cast(),bytes.len()) };
        if count>0 { bytes=&bytes[count as usize..]; continue; }
        let error=std::io::Error::last_os_error();
        if !matches!(error.kind(),std::io::ErrorKind::WouldBlock|std::io::ErrorKind::Interrupted) { return Err(io(error)); }
        if SIGNAL.load(Ordering::Relaxed)!=0 || std::time::Instant::now()>=deadline {
            return Err(Error::new("E_OUTPUT_IO",8,"watch stdout consumer did not accept its bounded event within 250 ms"));
        }
        thread::sleep(Duration::from_millis(2));
    }
    Ok(())
}
crate::schema::object_struct! {
    #[serde(rename_all="camelCase", deny_unknown_fields)]
    struct BuildStatus {
        schema_version: u32, attempt_id: String, app: String, status: String,
        previous_directory: Option<PathBuf>, publication_directory: Option<PathBuf>,
        build_id: Option<String>, error: Option<BuildError>
    }
}
crate::schema::object_struct! {
    #[serde(deny_unknown_fields)]
    struct BuildError { code: String, message: String }
}

#[derive(Clone, PartialEq)]
struct Stamp { inode: u64, device: u64, modified: std::time::SystemTime, length: u64 }
struct StatusReader { path: PathBuf, stamp: Option<Stamp>, value: Option<BuildStatus> }
impl StatusReader {
    fn refresh(&mut self) -> Result<bool, Error> {
        use std::os::unix::fs::MetadataExt;
        let metadata = match fs::symlink_metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind()==std::io::ErrorKind::NotFound => {
                let changed=self.stamp.take().is_some(); self.value=None; return Ok(changed);
            }
            Err(error) => return Err(io(error)),
        };
        let stamp=Stamp { inode:metadata.ino(),device:metadata.dev(),modified:metadata.modified().map_err(io)?,length:metadata.len() };
        if self.stamp.as_ref()==Some(&stamp) { return Ok(false); }
        let parent=self.path.parent().filter(|parent|!parent.as_os_str().is_empty()).unwrap_or(Path::new(".")).canonicalize().map_err(io)?;
        let bytes=crate::bounded_fs::read_contained(&parent,STATUS_FILE,16*1024)
            .map_err(|error|output::bounded_read_error(error,"build status is not a bounded regular file"))?;
        let status:BuildStatus=serde_json::from_slice(&bytes).map_err(|error|schema(format!("invalid build status: {error}")))?;
        if status.schema_version!=1 || status.attempt_id.len()!=36 || !status.attempt_id.bytes().all(|byte|byte.is_ascii_hexdigit() || byte==b'-')
            || status.app.is_empty() || status.app.len()>128 || !status.app.bytes().all(|byte|byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte==b'-')
            || !matches!(status.status.as_str(),"building"|"published"|"failed")
            || status.previous_directory.as_ref().is_some_and(|path|!path.is_absolute())
            || status.publication_directory.as_ref().is_some_and(|path|!path.is_absolute())
            || status.build_id.as_deref().is_some_and(|id|!build_id(id))
            || (status.status=="published" && (status.publication_directory.is_none() || status.build_id.is_none() || status.error.is_some()))
            || (status.status!="published" && (status.publication_directory.is_some() || status.build_id.is_some()))
            || (status.status=="failed")!=status.error.is_some()
            || status.error.as_ref().is_some_and(|error|error.code!="E_BUILD" || error.message.chars().count()>2048) {
            return Err(schema("invalid build status identity or state"));
        }
        self.stamp=Some(stamp); self.value=Some(status); Ok(true)
    }
}

struct Candidate { directory:PathBuf, build_id:String, run_id:String, relative:String, epoch:u64, build_attempt:Option<String> }
impl Candidate {
    fn index(&self, status:&str) -> Value {
        json!({"runId":self.run_id,"nativeRunId":null,"buildId":self.build_id,"buildAttemptId":self.build_attempt,
            "publicationDirectory":self.directory,"output":self.relative,"status":status})
    }
}
struct Active { candidate:Candidate, canceled:Arc<AtomicBool>, task:Option<JoinHandle<Result<Value,Error>>> }
impl Drop for Active {
    fn drop(&mut self) {
        self.canceled.store(true,Ordering::Release);
        if let Some(task)=self.task.take() { let _=task.join(); }
    }
}

struct WatchOutput {
    root:PathBuf, events:File, event_count:u64, event_limit:u64,
    epoch:u64, latest:Value, success:Value,
}
impl WatchOutput {
    fn create(path:&Path, max_runs:u64, per_run_limit:u64) -> Result<Self,Error> {
        // Reuse the one-shot exclusive ownership claim; the watch root has an
        // index, not a misleading run-artifact inventory over mutable pointers.
        let owner=Output::create(path,per_run_limit)?;
        let root=owner.root().to_path_buf();
        fs::create_dir(root.join("runs")).map_err(io)?;
        let events=OpenOptions::new().write(true).create_new(true).open(root.join("events.jsonl")).map_err(io)?;
        Ok(Self { root,events,event_count:0,event_limit:max_runs*16+64,epoch:0,latest:Value::Null,success:Value::Null })
    }
    fn emit(&mut self, event:&str, mut value:Value) -> Result<(),Error> {
        if self.event_count>=self.event_limit { return Err(Error::new("E_WATCH_LIMIT",5,"watch event retention limit reached; start a new output directory")); }
        value["schemaVersion"]=json!(1); value["event"]=json!(event); value["sequence"]=json!(self.event_count+1);
        let mut bytes=serde_json::to_vec(&value).map_err(|error|schema(error.to_string()))?; bytes.push(b'\n');
        if bytes.len()>super::MAX_STDOUT_BYTES { return Err(Error::new("E_OUTPUT_LIMIT",8,"watch event exceeds 8 KiB")); }
        self.events.write_all(&bytes).map_err(io)?; self.events.flush().map_err(io)?;
        self.event_count+=1;
        stdout(&bytes)
    }
    fn index(&self) -> Result<(),Error> {
        let mut success=self.success.clone();
        if !success.is_null() { success["historical"]=json!(self.latest["runId"]!=success["runId"] || self.latest["status"]!="passed"); }
        self.atomic("index.json",&json!({"schemaVersion":1,"revision":self.epoch,"latestAttempt":self.latest,"lastSuccess":success}))
    }
    fn atomic(&self, name:&str, value:&Value) -> Result<(),Error> {
        let bytes=output::json_bytes(value)?;
        if bytes.len()>16*1024 { return Err(Error::new("E_OUTPUT_LIMIT",8,"watch index exceeds 16 KiB")); }
        let temporary=self.root.join(format!(".{name}.tmp"));
        let mut file=OpenOptions::new().write(true).create_new(true).open(&temporary).map_err(io)?;
        file.write_all(&bytes).map_err(io)?; file.sync_all().map_err(io)?; drop(file);
        fs::rename(temporary,self.root.join(name)).map_err(io)
    }
    fn complete(&mut self, mut active:Active) -> Result<u8,Error> {
        let result=active.task.take().expect("owned run controller").join()
            .unwrap_or_else(|_|Err(Error::new("E_WORKER_CRASH",9,"run controller panicked")))
            .unwrap_or_else(|error|super::failure("run",error));
        let mut value=active.candidate.index(result["status"].as_str().unwrap_or("failed"));
        let mut exit=result["exitCode"].as_u64().unwrap_or(9) as u8;
        value["nativeRunId"]=result["runId"].clone(); value["exitCode"]=json!(exit);
        value["teardownVerified"]=result["teardown"]["verified"].clone();
        value["teardownClean"]=result["teardown"]["clean"].clone();
        value["firstError"]=compact_error(&result["firstError"]);
        if exit==0 && (result["status"]!="passed" || result["build"]["buildId"]!=active.candidate.build_id) {
            exit=9; value["exitCode"]=json!(exit); value["status"]=json!("failed");
            value["firstError"]=json!({"code":"E_HOST_INVARIANT","message":"worker success does not identify its accepted build"});
        }
        value["superseded"]=json!(self.epoch!=active.candidate.epoch);
        let canceled=result["cancellation"]["requested"]==true;
        if canceled { value["cancellation"]=result["cancellation"].clone(); }
        if exit==0 && self.epoch==active.candidate.epoch { self.success=value.clone(); }
        if self.epoch==active.candidate.epoch { self.latest=value.clone(); }
        self.index()?;
        self.emit(if canceled { "run.canceled" } else { "run.completed" },value)?;
        Ok(exit)
    }
}

pub(super) fn command(args:Vec<String>) -> u8 {
    let _stdio=match NonblockingOutput::install() { Ok(guard)=>guard,Err(_)=>return 8 };
    match execute(args) {
        Ok(code)=>code,
        Err(error)=>{
            let mut value=super::failure("watch",error); value["event"]=json!("watch.failed");
            let code=value["exitCode"].as_u64().unwrap_or(9) as u8;
            let _=stdout(format!("{value}\n").as_bytes()); code
        }
    }
}

fn execute(args:Vec<String>) -> Result<u8,Error> {
    let mut common=Vec::new(); let mut maximum=None; let mut args=args.into_iter();
    while let Some(key)=args.next() {
        let value=args.next().ok_or_else(||schema(format!("missing value for {key}")))?;
        if key=="--max-runs" {
            if maximum.is_some() { return Err(schema("duplicate --max-runs")); }
            let count=value.parse::<u64>().map_err(|_|schema("invalid --max-runs"))?;
            if !(1..=1024).contains(&count) { return Err(schema("--max-runs must be in 1..=1024")); }
            maximum=Some(count);
        } else { common.push(key); common.push(value); }
    }
    let maximum=maximum.unwrap_or(64);
    let run::ValidatedRun { options, profile, .. } = run::validate(&common)?;
    let bundle=std::path::absolute(&options.bundle).map_err(io)?;
    let status_path=bundle.parent().ok_or_else(||schema("bundle publication has no parent"))?.join(STATUS_FILE);
    let _signals=Signals::install()?;
    let mut output=WatchOutput::create(options.common.out.as_deref().expect("validated output"),maximum,profile.limits.max_output_bytes)?;
    output.index()?;
    output.emit("watch.started",json!({"bundle":bundle,"maxRuns":maximum,"eventLimit":output.event_limit,
        "perRunOutputLimitBytes":profile.limits.max_output_bytes,"index":"index.json"}))?;
    let mut reader=StatusReader { path:status_path,stamp:None,value:None };
    let mut observed=None::<PathBuf>; let mut unavailable=None::<String>;
    let mut pending=None::<Candidate>; let mut active=None::<Active>;
    let mut accepted=0_u64; let mut started=0_u64;
    let outcome=(|| -> Result<&str,Error> {
        loop {
            if SIGNAL.load(Ordering::Relaxed)!=0 { return Ok("signal"); }
            let changed=reader.refresh()?;
            let directory=match fs::symlink_metadata(&bundle) {
                Ok(metadata) if metadata.file_type().is_symlink()=>Some(fs::canonicalize(&bundle).map_err(io)?),
                Ok(_)=>return Err(schema("watch --bundle must be an atomically published generation symlink")),
                Err(error) if error.kind()==std::io::ErrorKind::NotFound=>None,
                Err(error)=>return Err(io(error)),
            };
            let blocked=reader.value.as_ref().filter(|status|status.status!="published" && status.previous_directory==directory);
            if let Some(status)=blocked {
                if changed {
                    output.epoch+=1;
                    output.latest=json!({"runId":null,"nativeRunId":null,"buildId":null,"buildAttemptId":status.attempt_id,
                        "status":status.status,"exitCode":if status.status=="failed" {json!(3)} else {Value::Null},
                        "publicationDirectory":null,"output":null,"firstError":status.error});
                    pending=None; output.index()?;
                    output.emit(if status.status=="failed" {"build.failed"} else {"build.started"},output.latest.clone())?;
                }
                observed=directory.clone();
            } else if let Some(directory)=directory {
                unavailable=None;
                if observed.as_ref()!=Some(&directory) {
                    // Resolve once and validate that immutable generation. Recheck
                    // publication after validation to coalesce intervening builds.
                    observed=Some(directory.clone());
                    let loaded=crate::package::Package::load(&directory);
                    if fs::canonicalize(&bundle).ok().as_ref()!=Some(&directory) { continue; }
                    output.epoch+=1;
                    match loaded {
                        Ok(package)=>{
                            accepted+=1;
                            let candidate=Candidate { directory,build_id:package.manifest.build_id.clone(),
                                run_id:format!("watch-{}-{accepted:06}",std::process::id()),relative:format!("runs/run-{accepted:06}"),epoch:output.epoch,
                                build_attempt:reader.value.as_ref().filter(|status|status.publication_directory.as_ref()==Some(&package.directory)).map(|status|status.attempt_id.clone()) };
                            output.latest=candidate.index("pending"); output.index()?;
                            output.emit("build.accepted",output.latest.clone())?;
                            if let Some(old)=&active { old.canceled.store(true,Ordering::Release); }
                            pending=Some(candidate);
                        }
                        Err(error)=>{
                            pending=None;
                            output.latest=json!({"runId":null,"nativeRunId":null,"buildId":null,"publicationDirectory":directory,
                                "status":"failed","exitCode":3,"output":null,"firstError":{"code":"E_BUNDLE","message":error.to_string().chars().take(1024).collect::<String>()}});
                            output.index()?; output.emit("build.rejected",output.latest.clone())?;
                        }
                    }
                }
            } else if unavailable.as_deref()!=Some("missing") {
                unavailable=Some("missing".into());
                output.emit("build.unavailable",json!({"message":"waiting for an atomic package publication"}))?;
            }
            if active.as_ref().is_some_and(|run|run.task.as_ref().is_some_and(|task|task.is_finished())) {
                output.complete(active.take().expect("finished controller"))?;
                if started>=maximum { return Ok("run-limit"); }
            }
            if active.is_none() && let Some(candidate)=pending.take() {
                let mut arguments=common.clone();
                for pair in arguments.as_chunks_mut::<2>().0 {
                    match pair[0].as_str() {
                        "--bundle"=>pair[1]=candidate.directory.to_string_lossy().into_owned(),
                        "--out"=>pair[1]=output.root.join(&candidate.relative).to_string_lossy().into_owned(),
                        _=>{}
                    }
                }
                let canceled=Arc::new(AtomicBool::new(false)); let token=canceled.clone();
                let task=thread::Builder::new().name("preview-run-controller".into()).spawn(move||run::controlled(arguments,Some(&token))).map_err(io)?;
                started+=1; output.latest=candidate.index("running");
                active=Some(Active { candidate,canceled,task:Some(task) });
                output.index()?; output.emit("run.started",output.latest.clone())?;
            }
            thread::sleep(POLL);
        }
    })();
    if let Some(run)=active.take() {
        run.canceled.store(true,Ordering::Release);
        let completion=output.complete(run);
        if outcome.is_ok() { completion?; }
    }
    match outcome {
        Ok(reason)=>{
            let code=if reason=="signal" {0} else {output.latest["exitCode"].as_u64().unwrap_or(5) as u8};
            let result=json!({"schemaVersion":1,"command":"watch","status":"stopped","reason":reason,"signal":SIGNAL.load(Ordering::Relaxed),
                "exitCode":code,"acceptedBuilds":accepted,"startedRuns":started,"index":"index.json"});
            output.atomic("watch.json",&result)?; output.emit("watch.stopped",result)?; Ok(code)
        }
        Err(error)=>{
            let result=super::failure("watch",error);
            let _=output.atomic("watch.json",&result);
            let code=result["exitCode"].as_u64().unwrap_or(9) as u8;
            let _=output.emit("watch.failed",result); Ok(code)
        }
    }
}
fn compact_error(value:&Value)->Value {
    if value.is_null() { Value::Null } else { json!({"code":value["code"],"message":value["message"].as_str().unwrap_or("unknown error").chars().take(1024).collect::<String>()}) }
}
fn build_id(value:&str)->bool { value.len()==71 && value.starts_with("sha256:") && value.as_bytes()[7..].iter().all(u8::is_ascii_hexdigit) }
fn schema(message:impl Into<String>)->Error { Error::new("E_SCHEMA",2,message) }
fn io(error:std::io::Error)->Error { Error::new("E_OUTPUT_IO",8,error.to_string()) }
