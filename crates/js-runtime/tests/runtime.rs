use js_runtime::{DiagnosticKind, JsRuntime, RuntimeLimits};
use rquickjs::Function;
use std::{cell::{Cell, RefCell}, rc::Rc, time::{Duration, Instant}};

fn runtime() -> JsRuntime {
    JsRuntime::new(RuntimeLimits::default()).unwrap()
}

#[test]
fn script_calls_native_function_and_preserves_exception() {
    let mut runtime = runtime();
    let text = Rc::new(RefCell::new(String::new()));
    let target = text.clone();
    runtime
        .with_context(|ctx| {
            let callback = Function::new(ctx.clone(), move |value: String| {
                *target.borrow_mut() = value;
            })?;
            ctx.globals().set("setNativeText", callback)
        })
        .unwrap();
    assert_eq!(
        runtime
            .evaluate_script("setNativeText('native text'); 'updated'")
            .unwrap(),
        "updated"
    );
    assert_eq!(&*text.borrow(), "native text");
    let error = runtime
        .evaluate_script("function broken() { throw new Error('script sentinel'); } broken();")
        .unwrap_err();
    assert_eq!(error.kind, DiagnosticKind::Exception);
    assert_eq!(error.message, "script sentinel");
    assert!(error.stack.as_deref().unwrap().contains("broken"));
    // The error is owned and does not poison subsequent context entries.
    assert_eq!(runtime.evaluate_script("'recovered'").unwrap(), "recovered");
    drop(runtime);
    assert_eq!(error.message, "script sentinel");
}

#[test]
fn arbitrary_thrown_string_is_retained() {
    let error = runtime()
        .evaluate_script("throw 'thrown string sentinel'")
        .unwrap_err();
    assert_eq!(error.message, "thrown string sentinel");
}

#[test]
fn module_declares_evaluates_waits_for_jobs_and_calls_export() {
    let mut runtime = runtime();
    let result = runtime
        .run_smoke_module(
            "jobs.mjs",
            "
        let ready = false;
        await Promise.resolve().then(() => { ready = true; });
        export function runSmoke() {
            if (!ready) throw new Error('module was not settled');
            return 'module:ok';
        }
    ",
        )
        .unwrap();
    assert_eq!(result, "module:ok");
}

#[test]
fn rejected_module_preserves_reason() {
    let error = runtime().run_smoke_module("rejected.mjs",
        "throw new Error('module rejection sentinel'); export function runSmoke() { return 'bad'; }")
        .unwrap_err();
    assert_eq!(error.kind, DiagnosticKind::Exception);
    assert_eq!(error.message, "module rejection sentinel");
}

#[test]
fn module_without_runnable_completion_is_not_busy_waited() {
    let error = runtime()
        .run_smoke_module(
            "pending.mjs",
            "await new Promise(() => {}); export function runSmoke() { return 'bad'; }",
        )
        .unwrap_err();
    assert_eq!(error.kind, DiagnosticKind::PendingModule);
}

#[test]
fn job_exception_retained_and_later_jobs_still_execute() {
    let mut runtime = runtime();
    runtime
        .with_context(|ctx| {
            let throwing: Function = ctx.eval("() => { throw new Error('job sentinel'); }")?;
            let succeeding: Function = ctx.eval("() => { globalThis.jobFinished = true; }")?;
            throwing.defer(())?;
            succeeding.defer(())
        })
        .unwrap();
    let turn = runtime.pump_jobs();
    assert_eq!(turn.attempted_jobs, 2);
    assert!(!turn.has_pending_jobs);
    assert_eq!(turn.diagnostics.len(), 1);
    assert_eq!(turn.diagnostics[0].phase, "pending job");
    assert_eq!(turn.diagnostics[0].message, "job sentinel");
    assert_eq!(
        runtime
            .evaluate_script("jobFinished ? 'yes' : 'no'")
            .unwrap(),
        "yes"
    );
}

#[test]
fn throwing_jobs_consume_count_budget() {
    let mut runtime = JsRuntime::new(RuntimeLimits {
        max_jobs_per_pump: 2,
        job_timeout: Duration::from_secs(1),
        ..RuntimeLimits::default()
    })
    .unwrap();
    runtime
        .with_context(|ctx| {
            let throwing: Function = ctx.eval("() => { throw 'count sentinel'; }")?;
            for _ in 0..5 {
                throwing.defer(())?;
            }
            Ok(())
        })
        .unwrap();
    let first = runtime.pump_jobs();
    assert_eq!(first.attempted_jobs, 2);
    assert_eq!(first.diagnostics.len(), 2);
    assert!(first.has_pending_jobs && first.budget_exhausted);
    let second = runtime.pump_jobs();
    assert_eq!(second.attempted_jobs, 2);
    let third = runtime.pump_jobs();
    assert_eq!(third.attempted_jobs, 1);
    assert!(!third.has_pending_jobs);
}

#[test]
fn infinite_script_is_interrupted_and_context_remains_usable() {
    let mut runtime = JsRuntime::new(RuntimeLimits {
        entry_timeout: Duration::from_millis(5),
        ..RuntimeLimits::default()
    })
    .unwrap();
    let error = runtime.evaluate_script("while (true) {} ").unwrap_err();
    assert_eq!(error.kind, DiagnosticKind::Interrupted);
    assert_eq!(
        runtime.evaluate_script("'after interrupt'").unwrap(),
        "after interrupt"
    );
}

#[test]
fn infinite_job_is_interrupted_inside_pump_deadline() {
    let mut runtime = JsRuntime::new(RuntimeLimits {
        entry_timeout: Duration::from_secs(1),
        job_timeout: Duration::from_millis(5),
        ..RuntimeLimits::default()
    })
    .unwrap();
    runtime
        .with_context(|ctx| {
            let forever: Function = ctx.eval("() => { while (true) {} }")?;
            forever.defer(())
        })
        .unwrap();
    let turn = runtime.pump_jobs();
    assert_eq!(turn.attempted_jobs, 1);
    assert_eq!(turn.diagnostics[0].kind, DiagnosticKind::Interrupted);
}

#[test]
fn exception_formatting_cannot_run_an_unbounded_getter() {
    let mut runtime = runtime();
    let error = runtime.evaluate_script("const evil = new Error(); Object.defineProperty(evil, 'message', { get() { while (true) {} } }); throw evil;").unwrap_err();
    assert_eq!(error.kind, DiagnosticKind::Exception);
    assert_eq!(
        runtime.evaluate_script("'still usable'").unwrap(),
        "still usable"
    );
}

#[test]
fn engine_memory_limit_is_effective_with_default_allocator() {
    let mut runtime = JsRuntime::new(RuntimeLimits {
        memory_limit_bytes: 4 * 1024 * 1024,
        entry_timeout: Duration::from_secs(2),
        ..RuntimeLimits::default()
    })
    .unwrap();
    let error = runtime.evaluate_script("const allocations = []; while (true) { allocations.push(new ArrayBuffer(64 * 1024)); }").unwrap_err();
    assert_ne!(
        error.kind,
        DiagnosticKind::Interrupted,
        "allocator must fail before the deadline: {error}"
    );
    assert!(error.message.to_lowercase().contains("memory"), "{error}");
}

#[test]
fn invalid_unbounded_limits_are_rejected() {
    assert!(matches!(JsRuntime::new(RuntimeLimits {
        max_jobs_per_pump: 0,
        ..RuntimeLimits::default()
    }), Err(error) if error.kind == DiagnosticKind::InvalidLimits));
}

#[test]
fn individual_jobs_report_failure_success_and_empty_without_extra_attempts() {
    let mut runtime = runtime();
    runtime.with_context(|ctx| {
        let throwing: Function = ctx.eval("() => { throw new Error('single job'); }")?;
        let succeeding: Function = ctx.eval("() => { globalThis.singleJobDone = true; }")?;
        throwing.defer(())?;
        succeeding.defer(())
    }).unwrap();
    assert!(runtime.is_job_pending());
    let error = runtime.execute_job(Duration::from_secs(1)).unwrap_err();
    assert_eq!(error.phase, "pending job");
    assert_eq!(error.message, "single job");
    assert!(runtime.is_job_pending());
    assert!(runtime.execute_job(Duration::from_secs(1)).unwrap());
    assert!(!runtime.is_job_pending());
    assert!(!runtime.execute_job(Duration::from_secs(1)).unwrap());
    assert_eq!(runtime.evaluate_script("String(singleJobDone)").unwrap(), "true");
    assert_eq!(runtime.limits().max_jobs_per_pump, 256);
}

#[test]
fn individual_job_interrupts_at_remaining_budget_and_leaves_next_job_runnable() {
    let mut runtime = JsRuntime::new(RuntimeLimits {
        entry_timeout: Duration::from_secs(2),
        ..RuntimeLimits::default()
    }).unwrap();
    runtime.with_context(|ctx| {
        let forever: Function = ctx.eval("() => { while (true) {} }")?;
        let succeeding: Function = ctx.eval("() => {}")?;
        forever.defer(())?;
        succeeding.defer(())
    }).unwrap();
    let start = Instant::now();
    let error = runtime.execute_job(Duration::from_millis(5)).unwrap_err();
    assert_eq!(error.kind, DiagnosticKind::Interrupted);
    assert!(start.elapsed() < Duration::from_secs(1));
    assert!(runtime.is_job_pending());
    assert!(runtime.execute_job(Duration::from_millis(5)).unwrap());
}

#[test]
fn individual_job_also_respects_normal_entry_deadline() {
    let mut runtime = JsRuntime::new(RuntimeLimits {
        entry_timeout: Duration::from_millis(5),
        ..RuntimeLimits::default()
    }).unwrap();
    runtime.with_context(|ctx| {
        let forever: Function = ctx.eval("() => { while (true) {} }")?;
        forever.defer(())
    }).unwrap();
    let start = Instant::now();
    assert_eq!(runtime.execute_job(Duration::from_secs(2)).unwrap_err().kind, DiagnosticKind::Interrupted);
    assert!(start.elapsed() < Duration::from_secs(1));
}

#[test]
fn cleanup_interrupts_under_its_own_deadline_and_owner_remains_usable() {
    let mut runtime = JsRuntime::new(RuntimeLimits {
        entry_timeout: Duration::from_secs(2),
        cleanup_timeout: Duration::from_millis(5),
        ..RuntimeLimits::default()
    }).unwrap();
    let start = Instant::now();
    let error = runtime.with_cleanup_context(|ctx| ctx.eval::<(), _>("while (true) {}")).unwrap_err();
    assert_eq!(error.kind, DiagnosticKind::Interrupted);
    assert_eq!(error.phase, "cleanup");
    assert!(start.elapsed() < Duration::from_secs(1));
    assert_eq!(runtime.evaluate_script("'after cleanup'").unwrap(), "after cleanup");
}

#[test]
fn rejection_later_handling_keeps_identity_and_original_source_diagnostic() {
    let mut runtime = runtime();
    runtime.evaluate_script("
        function rejectAtSource() {
            globalThis.reason = new Error('rejection source');
            globalThis.rejected = Promise.reject(reason);
        }
        rejectAtSource(); 'queued'
    ").unwrap();
    let first = runtime.drain_rejections();
    assert_eq!(first.dropped_rejections, 0);
    assert_eq!(first.updates.len(), 1);
    let first = &first.updates[0];
    assert!(!first.handled);
    assert_eq!(first.diagnostic.phase, "promise rejection");
    assert_eq!(first.diagnostic.message, "rejection source");
    assert!(first.diagnostic.stack.as_deref().unwrap().contains("rejectAtSource"));
    assert!(runtime.drain_rejections().updates.is_empty());
    runtime.evaluate_script("
        Object.defineProperty(reason, 'message', { get() { throw new Error('must not reformat'); } });
        rejected.catch(() => {}); 'handled'
    ").unwrap();
    let handled = runtime.drain_rejections();
    assert_eq!(handled.updates.len(), 1);
    assert!(handled.updates[0].handled);
    assert_eq!(handled.updates[0].id, first.id);
    assert_eq!(handled.updates[0].diagnostic, first.diagnostic);
    drop(runtime);
    assert_eq!(handled.updates[0].diagnostic.message, "rejection source");
}

#[test]
fn rejection_handled_before_drain_retains_both_transitions() {
    let mut runtime = runtime();
    runtime.evaluate_script("const transient = Promise.reject('transient'); transient.catch(() => {}); 'ok'").unwrap();
    let report = runtime.drain_rejections();
    assert_eq!(report.updates.len(), 2);
    assert!(!report.updates[0].handled);
    assert!(report.updates[1].handled);
    assert_eq!(report.updates[0].id, report.updates[1].id);
    assert_eq!(report.updates[0].diagnostic, report.updates[1].diagnostic);
}

#[test]
fn rejection_formatting_can_create_and_handle_rejections_without_reentrant_borrows() {
    let mut runtime = runtime();
    runtime.evaluate_script("
        const outerReason = new Error();
        Object.defineProperty(outerReason, 'message', { get() {
            const nested = Promise.reject('nested rejection');
            nested.catch(() => {});
            return 'outer rejection';
        } });
        Promise.reject(outerReason); 'queued'
    ").unwrap();
    let first = runtime.drain_rejections();
    assert_eq!(first.updates.len(), 1);
    assert_eq!(first.updates[0].diagnostic.message, "outer rejection");
    let nested = runtime.drain_rejections();
    assert!(nested.updates.iter().any(|update| !update.handled && update.diagnostic.message == "nested rejection"));
    assert!(nested.updates.iter().any(|update| update.handled && update.diagnostic.message == "nested rejection"));
    assert_eq!(runtime.evaluate_script("'usable'").unwrap(), "usable");
}

#[test]
fn rejection_formatting_interrupts_getters_and_bounds_owned_text() {
    let mut runtime = JsRuntime::new(RuntimeLimits {
        max_diagnostic_bytes: 24,
        diagnostic_timeout: Duration::from_millis(5),
        ..RuntimeLimits::default()
    }).unwrap();
    runtime.evaluate_script("
        const evilReason = new Error();
        Object.defineProperty(evilReason, 'message', { get() { while (true) {} } });
        Promise.reject(evilReason);
        Promise.reject('é'.repeat(1000)); 'queued'
    ").unwrap();
    let start = Instant::now();
    let report = runtime.drain_rejections();
    assert!(start.elapsed() < Duration::from_secs(1));
    assert_eq!(report.updates.len(), 2);
    for update in report.updates {
        assert!(update.diagnostic.message.len() <= 24);
        assert!(update.diagnostic.stack.as_ref().is_none_or(|stack| stack.len() <= 24));
    }
    assert_eq!(runtime.evaluate_script("'usable'").unwrap(), "usable");
}

#[test]
fn rejection_capacity_reports_eviction_and_drop_releases_undrained_roots() {
    for drain in [true, false] {
        let mut runtime = JsRuntime::new(RuntimeLimits {
            max_tracked_rejections: 2,
            ..RuntimeLimits::default()
        }).unwrap();
        runtime.evaluate_script("for (let i = 0; i < 8; i++) Promise.reject(new Error('rejected ' + i)); 'queued'").unwrap();
        if drain {
            let report = runtime.drain_rejections();
            assert_eq!(report.dropped_rejections, 6);
            assert_eq!(report.updates.len(), 2);
            assert_eq!(report.updates[0].diagnostic.message, "rejected 6");
            assert_eq!(report.updates[1].diagnostic.message, "rejected 7");
            assert_eq!(runtime.drain_rejections().dropped_rejections, 0);
        }
        drop(runtime);
    }
}

#[test]
fn throwing_job_and_cleanup_do_not_extend_deadlines_to_format_accessors() {
    for cleanup in [false, true] {
        let mut runtime = JsRuntime::new(RuntimeLimits {
            entry_timeout: Duration::from_secs(2),
            cleanup_timeout: Duration::from_millis(5),
            diagnostic_timeout: Duration::from_secs(1),
            ..RuntimeLimits::default()
        }).unwrap();
        runtime.evaluate_script("globalThis.evil = new Error(); Object.defineProperty(evil, 'message', { get() { while (true) {} } }); 'ready'").unwrap();
        if !cleanup {
            runtime.with_context(|ctx| ctx.eval::<Function, _>("() => { throw evil; }")?.defer(())).unwrap();
        }
        let start = Instant::now();
        let error = if cleanup {
            runtime.with_cleanup_context(|ctx| ctx.eval::<(), _>("throw evil")).unwrap_err()
        } else {
            runtime.execute_job(Duration::from_millis(5)).unwrap_err()
        };
        assert_eq!(error.kind, DiagnosticKind::Exception);
        assert!(start.elapsed() < Duration::from_millis(500), "diagnostic formatting extended a five-millisecond entry");
        assert_eq!(runtime.evaluate_script("'usable'").unwrap(), "usable");
    }
}

#[test]
fn self_replenishing_microtask_chain_yields_and_can_be_cancelled_by_next_entry() {
    let mut runtime = JsRuntime::new(RuntimeLimits {
        max_jobs_per_pump: 8,
        job_timeout: Duration::from_secs(1),
        ..RuntimeLimits::default()
    }).unwrap();
    runtime.evaluate_script("globalThis.running = true; globalThis.turns = 0; function again() { ++turns; if (running) queueMicrotask(again); } queueMicrotask(again); 'armed'").unwrap();
    let pump = runtime.pump_jobs();
    assert_eq!(pump.attempted_jobs, 8);
    assert!(pump.has_pending_jobs && pump.budget_exhausted);
    assert!(pump.diagnostics.is_empty());
    assert_eq!(runtime.evaluate_script("running = false; String(turns)").unwrap(), "8");
    let pump = runtime.pump_jobs();
    assert_eq!(pump.attempted_jobs, 1);
    assert!(!pump.has_pending_jobs);
}

#[test]
fn owner_deadline_clamps_entries_jobs_and_cleanup_and_can_be_renewed() {
    let mut runtime = runtime();
    runtime.with_context(|ctx| ctx.eval::<Function, _>("() => { globalThis.jobRan = true; }")?.defer(())).unwrap();
    let deadline = Rc::new(Cell::new(Instant::now() - Duration::from_millis(1)));
    runtime.set_enclosing_deadline(deadline.clone());
    let error = runtime.evaluate_script("globalThis.scriptRan = true; 'ran'").unwrap_err();
    assert_eq!(error.kind, DiagnosticKind::BudgetExceeded);
    assert_eq!(runtime.execute_job(Duration::from_secs(1)).unwrap_err().kind, DiagnosticKind::BudgetExceeded);
    assert!(runtime.is_job_pending(), "an expired deadline may not consume the queued job");
    let error = runtime.with_cleanup_context(|ctx| ctx.eval::<(), _>("globalThis.cleanupRan = true")).unwrap_err();
    assert_eq!(error.phase, "cleanup");
    assert_eq!(error.kind, DiagnosticKind::BudgetExceeded);
    deadline.set(Instant::now() + Duration::from_secs(1));
    assert_eq!(runtime.evaluate_script("String(globalThis.scriptRan) + '/' + String(globalThis.cleanupRan)").unwrap(), "undefined/undefined");
    assert!(runtime.execute_job(Duration::from_secs(1)).unwrap());
    assert_eq!(runtime.evaluate_script("String(jobRan)").unwrap(), "true");
}

#[test]
fn owner_deadline_interrupts_running_script_job_and_cleanup() {
    for phase in ["script", "job", "cleanup", "module"] {
        let mut runtime = JsRuntime::new(RuntimeLimits {
            entry_timeout: Duration::from_secs(2),
            cleanup_timeout: Duration::from_secs(2),
            job_timeout: Duration::from_secs(2),
            ..RuntimeLimits::default()
        }).unwrap();
        if phase == "job" {
            runtime.with_context(|ctx| ctx.eval::<Function, _>("() => { while (true) {} }")?.defer(())).unwrap();
        }
        let deadline = Rc::new(Cell::new(Instant::now() + Duration::from_millis(5)));
        runtime.set_enclosing_deadline(deadline.clone());
        let start = Instant::now();
        let error = match phase {
            "script" => runtime.evaluate_script("while (true) {}").unwrap_err(),
            "job" => runtime.execute_job(Duration::from_secs(2)).unwrap_err(),
            "cleanup" => runtime.with_cleanup_context(|ctx| ctx.eval::<(), _>("while (true) {}")).unwrap_err(),
            _ => runtime.load_module("deadline.mjs", "while (true) {}").unwrap_err(),
        };
        assert!(matches!(error.kind, DiagnosticKind::Interrupted | DiagnosticKind::BudgetExceeded), "{phase}: {error}");
        assert!(start.elapsed() < Duration::from_millis(500), "{phase} exceeded the enclosing real deadline");
        deadline.set(Instant::now() + Duration::from_secs(1));
        assert_eq!(runtime.evaluate_script("'renewed'").unwrap(), "renewed");
    }
}

#[test]
fn rejection_accessors_cannot_extend_an_expired_owner_deadline() {
    let mut runtime = runtime();
    runtime.evaluate_script("globalThis.readMessage = false; const reason = new Error(); Object.defineProperty(reason, 'message', { get() { readMessage = true; while (true) {} } }); Promise.reject(reason); 'queued'").unwrap();
    let deadline = Rc::new(Cell::new(Instant::now() - Duration::from_millis(1)));
    runtime.set_enclosing_deadline(deadline.clone());
    let drain = runtime.drain_rejections();
    assert_eq!(drain.updates.len(), 1);
    deadline.set(Instant::now() + Duration::from_secs(1));
    assert_eq!(runtime.evaluate_script("String(readMessage)").unwrap(), "false");
}

#[inline(never)]
fn at_deep_rust_stack<R>(depth: usize, f: impl FnOnce() -> R) -> R {
    let mut frame = std::hint::black_box([0u8; 8192]);
    std::hint::black_box(frame.as_mut_ptr());
    if depth == 0 {
        f()
    } else {
        let result = at_deep_rust_stack(depth - 1, f);
        std::hint::black_box(frame[0]);
        result
    }
}

#[test]
fn stack_limit_measures_from_entry_not_creation_depth_and_still_bounds_js_recursion() {
    let mut runtime = JsRuntime::new(RuntimeLimits {
        entry_timeout: Duration::from_secs(2),
        cleanup_timeout: Duration::from_secs(2),
        job_timeout: Duration::from_secs(2),
        ..RuntimeLimits::default()
    })
    .unwrap();
    // 40 * 8 KiB = 320 KiB > 256 KiB stack_limit_bytes relative to creation depth.
    at_deep_rust_stack(40, || {
        assert_eq!(runtime.evaluate_script("'ok'").unwrap(), "ok");
        runtime
            .with_context(|ctx| {
                ctx.eval::<Function, _>("() => { Promise.reject(new Error('deep')); }")?.defer(())
            })
            .unwrap();
        assert!(runtime.execute_job(Duration::from_secs(1)).unwrap());
        assert_eq!(runtime.drain_rejections().updates.len(), 1);
        runtime.with_cleanup_context(|ctx| ctx.eval::<(), _>("'cleaned'")).unwrap();

        let err = runtime
            .evaluate_script("(function f() { return 1 + f(); })()")
            .unwrap_err();
        assert_eq!(err.kind, DiagnosticKind::Exception);
        assert!(
            err.message.to_lowercase().contains("stack"),
            "expected JS stack overflow diagnostic, got: {err}"
        );
    });
}
