//! Application time reads the native platform clock; execution budgets never do.
use std::{cell::Cell,rc::Rc,sync::Arc};
use rquickjs::{Ctx,Function};
use serde_json::{Value,json};
use zgui::platform::Clock;

pub(crate) struct Environment {
    clock:Arc<dyn Clock>,
    epoch_ms:Option<u64>,
    random:Cell<u64>,
}
impl Environment {
    pub(crate) fn new(clock:Arc<dyn Clock>,epoch_ms:Option<u64>,seed:u32)->Rc<Self> {
        Rc::new(Self{clock,epoch_ms,random:Cell::new(u64::from(seed))})
    }
    pub(crate) fn now_us(&self)->u64 { self.clock.timestamp().since_origin().as_micros() as u64 }
    pub(crate) fn is_virtual(&self)->bool { self.epoch_ms.is_some() }
    pub(crate) fn configuration(epoch_ms:Option<u64>,seed:u32)->Value {
        json!({"revision":"native-clock-date-v1","mode":if epoch_ms.is_some(){"virtual"}else{"wall"},
            "epochMs":epoch_ms,"timeZone":"UTC","date":if epoch_ms.is_some(){"virtual-utc"}else{"ambient-real"},
            "performanceNow":"native-elapsed-ms","random":{"algorithm":"splitmix64-v1","seed":seed}})
    }
    pub(crate) fn install(self:&Rc<Self>,ctx:&Ctx<'_>)->rquickjs::Result<()> {
        let elapsed=self.clone();
        let performance=Function::new(ctx.clone(),move ||elapsed.clock.timestamp().since_origin().as_secs_f64()*1000.0)?;
        let wall=self.clone();
        // Date has integer millisecond precision; performance retains the submillisecond part.
        let date=Function::new(ctx.clone(),move ||(wall.epoch_ms.unwrap_or(0)+wall.now_us()/1000) as f64)?;
        let random=self.clone();
        let random=Function::new(ctx.clone(),move ||random.next_random())?;
        let install:Function=ctx.eval(INSTALL)?;
        install.call((self.is_virtual(),date,performance,random))
    }
    fn next_random(&self)->f64 {
        let state=self.random.get().wrapping_add(0x9e3779b97f4a7c15);
        self.random.set(state);
        let mut value=state;
        value=(value^(value>>30)).wrapping_mul(0xbf58476d1ce4e5b9);
        value=(value^(value>>27)).wrapping_mul(0x94d049bb133111eb);
        value^=value>>31;
        (value>>11) as f64*(1.0/9007199254740992.0)
    }
}

const INSTALL:&str=r#"((virtual, now, monotonic, random) => {
    globalThis.performance = Object.freeze({ now: monotonic });
    Object.defineProperty(Math, 'random', { value: random, writable: true, configurable: true });
    if (!virtual) return;
    const NativeDate = globalThis.Date;
    function Date(...args) {
        if (new.target) return Reflect.construct(NativeDate, args.length ? args : [now()], new.target);
        return new NativeDate(now()).toString();
    }
    Object.defineProperties(Date, {
        parse: { value: NativeDate.parse, writable: true, configurable: true },
        UTC: { value: NativeDate.UTC, writable: true, configurable: true }
    });
    Date.prototype = NativeDate.prototype;
    Object.defineProperty(Date, 'length', { value: 7, configurable: true });
    Object.defineProperty(Date, 'now', { value: now, writable: true, configurable: true });
    Object.defineProperty(Date.prototype, 'constructor', { value: Date, writable: true, configurable: true });
    globalThis.Date = Date;
})"#;

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use js_runtime::{JsRuntime,DiagnosticKind,RuntimeLimits};
    use zgui::platform::VirtualClock;

    fn runtime(clock:Arc<VirtualClock>,seed:u32)->JsRuntime {
        let environment=Environment::new(clock,Some(1_700_000_000_000),seed);
        let mut engine=JsRuntime::new(RuntimeLimits::default()).unwrap();
        engine.with_context(|ctx|environment.install(&ctx)).unwrap();
        engine
    }
    #[test]
    fn date_construction_call_and_static_apis_share_native_time() {
        let clock=Arc::new(VirtualClock::new());
        clock.advance(Duration::from_micros(1250));
        let mut engine=runtime(clock.clone(),7);
        engine.with_context(|ctx|ctx.eval::<(),_>(r#"
            if(Date.now()!==1700000000001 || new Date().getTime()!==Date.now()) throw Error('clock disagreement');
            if(performance.now()!==1.25) throw Error('lost microseconds');
            if(Date()!==new Date().toString() || Date('ignored')!==Date()) throw Error('callable Date');
            if(new Date(undefined).getTime()===new Date().getTime()) throw Error('argument construction');
            if(Date.parse('2000-01-01T00:00:00Z')!==946684800000 || Date.UTC(2000,0,1)!==946684800000) throw Error('static APIs');
            if(new Date(123).getTime()!==123 || new Date().constructor!==Date || !(new Date() instanceof Date)) throw Error('constructor identity');
            class ChildDate extends Date {};
            const child=new ChildDate();
            if(!(child instanceof ChildDate) || child.getTime()!==Date.now()) throw Error('subclass construction');
            globalThis.original=Date.now();
        "#)).unwrap();
        clock.advance(Duration::from_micros(9750));
        engine.with_context(|ctx|ctx.eval::<(),_>("if(Date.now()!==original+10 || performance.now()!==11) throw Error('native advance not visible')")).unwrap();
    }
    #[test]
    fn seeded_random_replays_across_contexts_without_changing_microtasks() {
        let mut first=runtime(Arc::new(VirtualClock::new()),0);
        let mut second=runtime(Arc::new(VirtualClock::new()),0);
        let sample="JSON.stringify(Array.from({length:32},()=>Math.random()))";
        let left:String=first.with_context(|ctx|ctx.eval(sample)).unwrap();
        let right:String=second.with_context(|ctx|ctx.eval(sample)).unwrap();
        assert_eq!(left,right);
        let values:Vec<f64>=serde_json::from_str(&left).unwrap();
        assert!(values.iter().all(|value|(0.0..1.0).contains(value)));
        assert!(values.windows(2).any(|values|values[0]!=values[1]));
        first.with_context(|ctx|ctx.eval::<(),_>("globalThis.order=[]; queueMicrotask(()=>order.push('microtask')); order.push('entry')")).unwrap();
        assert!(first.pump_jobs().diagnostics.is_empty());
        let order:String=first.with_context(|ctx|ctx.eval("JSON.stringify(order)")).unwrap();
        assert_eq!(order,"[\"entry\",\"microtask\"]");
    }
    #[test]
    fn real_interrupt_stops_infinite_javascript_while_native_time_is_frozen() {
        let clock=Arc::new(VirtualClock::new());
        let mut engine=runtime(clock.clone(),1);
        let error=engine.with_context(|ctx|ctx.eval::<(),_>("for(;;){}" )).unwrap_err();
        assert_eq!(error.kind,DiagnosticKind::Interrupted);
        assert_eq!(clock.timestamp().since_origin(),Duration::ZERO);
    }
}
