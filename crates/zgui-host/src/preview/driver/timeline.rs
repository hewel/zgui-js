//! Explicit timeline changes; no fixed frame increments or hidden virtual-time advances.
use super::{Driver,Fault};
use super::super::{Error,profile,scenario::ServiceCallCheck};
use serde_json::{Value,json};
use std::time::Duration;
use zgui::{geom::{DevicePx,Size},platform::{PlatformCx,SurfaceEvent}};

impl Driver {
    pub fn advance(&mut self,by_ms:u64)->Result<Value,Fault> {
        if !self.environment.is_virtual() { return Err(Error::new("E_UNSUPPORTED_CAPABILITY",2,"clock.advance requires a virtual profile").into()); }
        let from=self.environment.now_us();
        let target=by_ms.checked_mul(1000).and_then(|delta|from.checked_add(delta))
            .filter(|target|*target<=self.maximum_time_us)
            .ok_or_else(||Error::new("E_VIRTUAL_TIME_BUDGET",5,"clock.advance exceeds maxVirtualDurationMs"))?;
        let origin=self.pipeline.platform().clock().origin();
        let endpoint=origin+Duration::from_micros(target);
        let first_frame=self.frames();
        self.settle(false)?;
        while self.pipeline.now()<endpoint {
            let now=self.pipeline.now();
            let mut next=endpoint;
            if let Some(deadline)=self.pipeline.parked_deadline().filter(|deadline|*deadline>now) { next=next.min(deadline); }
            if let Some(deadline)=self.fixtures.next_deadline_us() {
                let deadline=origin+Duration::from_micros(deadline);
                if deadline>now { next=next.min(deadline); }
            }
            self.pipeline.advance(next.saturating_duration_since(now));
            self.settle(false)?;
        }
        Ok(json!({"fromTimeUs":from,"timeUs":self.environment.now_us(),"advancedUs":target-from,
            "nativeFrames":self.frames()-first_frame,"readiness":self.readiness()}))
    }
    pub fn emit(&mut self,channel:&str,value:&Value)->Result<Value,Fault> {
        self.settle(false)?;
        self.fixtures.emit(channel,value)?;
        self.notify_fixture();
        self.settle(false)?;
        Ok(json!({"channel":channel,"timeUs":self.environment.now_us(),"readiness":self.readiness()}))
    }
    pub fn assert_service(&mut self,check:&ServiceCallCheck)->Result<Value,Fault> {
        self.settle(false)?;
        self.fixtures.assert_calls(&check.service,&check.method,check.count,check.error_code.as_deref())?;
        Ok(json!({"serviceCall":check,"timeUs":self.environment.now_us()}))
    }
    pub fn resize(&mut self,name:&str,width:u32,height:u32)->Result<Value,Fault> {
        let (_, _, scale) = self.geometry(name)?;
        self.configure_surface(name,f64::from(width),f64::from(height),scale,false)
    }
    pub fn scale(&mut self,name:&str,scale:f64)->Result<Value,Fault> {
        let (width,height,_) = self.geometry(name)?;
        self.configure_surface(name,width,height,scale,true)
    }
    pub fn color_scheme(&mut self,name:&str,scheme:&str)->Result<Value,Fault> {
        self.settle(false)?;
        let label = scheme;
        let scheme=match label {
            "light"=>zgui_style::ColorScheme::Light,
            "dark"=>zgui_style::ColorScheme::Dark,
            _=>return Err(Error::new("E_SCHEMA",2,"colorScheme must be light or dark").into()),
        };
        let window = self.window(name)?;
        let changed=window.color_scheme()!=scheme;
        if changed {
            self.observer(window.surface().id())?.bump_surface_configuration();
            self.window_mut(name)?.set_color_scheme(scheme);
            self.window(name)?.request_frame();
        }
        self.pump_application()?;
        self.settle(false)?;
        Ok(json!({"colorScheme":label,"timeUs":self.environment.now_us(),"readiness":self.readiness_for(Some(name))}))
    }
    fn geometry(&self,name:&str)->Result<(f64,f64,f64),Error> {
        if name == "main" {
            self.window(name)?;
            return Ok((f64::from(self.surface.width),f64::from(self.surface.height),self.surface.scale));
        }
        let surface=self.window(name)?.surface();
        let size=surface.size();
        let scale=surface.scale_factor();
        Ok((f64::from(size.width.0)/scale,f64::from(size.height.0)/scale,scale))
    }
    fn configure_surface(&mut self,name:&str,width:f64,height:f64,scale:f64,scale_changed:bool)->Result<Value,Fault> {
        self.settle(false)?;
        let previous = self.geometry(name)?;
        let dimensions=profile::physical_dimensions(width,height,scale)?;
        let surface=self.window(name)?.surface().id();
        if name=="main" {
            self.surface.width=width as u32;
            self.surface.height=height as u32;
            self.surface.scale=scale;
            self.dimensions=dimensions;
        }
        let size=Size::new(DevicePx(dimensions.0 as f32),DevicePx(dimensions.1 as f32));
        let changed=previous!=(width,height,scale);
        if changed { self.observer(surface)?.bump_surface_configuration(); }
        self.pipeline.deliver(surface,if scale_changed { SurfaceEvent::ScaleFactorChanged{scale_factor:scale,size} } else { SurfaceEvent::Resized(size) });
        if changed { self.window(name)?.request_frame(); }
        // Deliver the real event without advancing the held native clock. Pacing may defer its frame.
        self.pump_application()?;
        self.settle(false)?;
        let (width,height)=if name=="main" { (json!(self.surface.width),json!(self.surface.height)) } else { (json!(width),json!(height)) };
        Ok(json!({"width":width,"height":height,"scale":scale,
            "physicalWidth":dimensions.0,"physicalHeight":dimensions.1,"timeUs":self.environment.now_us(),"readiness":self.readiness_for(Some(name))}))
    }
}
