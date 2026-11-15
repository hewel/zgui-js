use serde_json::{Value,json};
use zgui::{geom::{Css,CssPx,Device,DevicePx,Point}, platform::{PlatformCx,SurfaceEvent}, view::NodeId,
    vocab::{ImeEvent,KeyEvent,KeyState,Modifiers,NamedKey,PhysicalKey,PointerAction,PointerEvent}};
use super::{Driver,Fault,OwnedFrame};
use super::super::{Error,scenario::{Check,Key,Modifier,PointerButton,PointerLocation,Target},selectors};

type DevicePoint = Point<DevicePx,Device>;

impl Driver {
    pub fn pointer(&mut self, location:&PointerLocation, button:Option<PointerButton>) -> Result<Value,Fault> {
        self.settle(true)?;
        let (position,mut trace) = match location {
            PointerLocation::Position{x,y,..} => {
                let position:Point<CssPx,Css>=Point::new(CssPx(*x as f32),CssPx(*y as f32));
                if !position.x.0.is_finite() || !position.y.0.is_finite() {
                    return Err(Error::new("E_SCHEMA",2,"pointer position exceeds native coordinate range").into());
                }
                let scale=self.window()?.surface().scale_factor();
                let (x,y)=(f64::from(position.x.0),f64::from(position.y.0));
                (position,json!({"positionLogical":{"x":x,"y":y},"positionDevice":{"x":x*scale,"y":y*scale},"targeted":false}))
            }
            PointerLocation::Target(target) => {
                let last=self.probe.last.borrow();
                let last=last.as_ref().ok_or_else(stale)?;
                let index=selectors::unique(&last.tree,target,false)?.expect("required unique target");
                let native=last.native_nodes.get(index).ok_or_else(stale)?.native;
                let record=&last.tree.nodes[index];
                if record.state.visible!=Some(true) {
                    return Err(target_error("E_TARGET_HIDDEN","target has no positive visible native box",record).into());
                }
                if record.state.enabled!=Some(true) {
                    return Err(target_error("E_TARGET_DISABLED","target or an ancestor is disabled",record).into());
                }
                let window=self.window()?;
                let (device,samples)=target_point(window,native,last)?;
                let scale=window.surface().scale_factor() as f32;
                let logical=Point::new(CssPx(device.x.0/scale),CssPx(device.y.0/scale));
                (logical,json!({"target":selectors::summary(record),"generation":last.tree.generation,
                    "validationFrame":serde_json::to_value(&last.frame.validation_frame).expect("frame stamp serializes"),"metadataRevision":last.metadata_revision,
                    "positionLogical":{"x":logical.x.0,"y":logical.y.0},"positionDevice":{"x":device.x.0,"y":device.y.0},
                    "nativeHitVerified":true,"pointSamples":samples}))
            }
        };
        if !self.frame_current() { return Err(stale().into()); }
        let surface=self.window()?.surface().id();
        let timestamp=self.harness.platform().clock().timestamp();
        let native_button=button.map(|button|match button {
            PointerButton::Primary=>zgui::vocab::PointerButton::Primary,
            PointerButton::Secondary=>zgui::vocab::PointerButton::Secondary,
            PointerButton::Middle=>zgui::vocab::PointerButton::Middle,
        });
        let event=PointerEvent::mouse(position);
        let event=if let Some(button)=native_button { event.with_button(button) } else { event };
        let actions:&[PointerAction]=if button.is_some() { &[PointerAction::Moved,PointerAction::Pressed,PointerAction::Released] } else { &[PointerAction::Moved] };
        self.harness.deliver_all(surface,actions.iter().map(|action|SurfaceEvent::Pointer {
            action:*action,event:event.clone(),modifiers:Modifiers::NONE,timestamp,
        }));
        self.harness.pump();
        self.check()?;
        self.settle(false)?;
        trace["burst"]=if button.is_some() { json!(["pointer.move","pointer.press","pointer.release"]) } else { json!(["pointer.move"]) };
        trace["button"]=json!(button);
        trace["timeUs"]=json!(timestamp.since_origin().as_micros());
        Ok(trace)
    }

    pub fn key(&mut self,key:Key,modifiers:&[Modifier]) -> Result<Value,Fault> {
        self.settle(true)?;
        let named=match key {
            Key::Tab=>NamedKey::Tab,Key::Enter=>NamedKey::Enter,Key::Escape=>NamedKey::Escape,Key::Space=>NamedKey::Space,
            Key::ArrowUp=>NamedKey::ArrowUp,Key::ArrowDown=>NamedKey::ArrowDown,Key::ArrowLeft=>NamedKey::ArrowLeft,Key::ArrowRight=>NamedKey::ArrowRight,
            Key::Home=>NamedKey::Home,Key::End=>NamedKey::End,Key::Backspace=>NamedKey::Backspace,Key::Delete=>NamedKey::Delete,
        };
        let held=modifiers.iter().fold(Modifiers::NONE,|held,modifier|held|match modifier {
            Modifier::Shift=>Modifiers::SHIFT,Modifier::Control=>Modifiers::CONTROL,Modifier::Alt=>Modifiers::ALT,Modifier::Meta=>Modifiers::META,
        });
        let surface=self.window()?.surface().id();
        let timestamp=self.harness.platform().clock().timestamp();
        let event=KeyEvent::named(named,PhysicalKey::Unidentified(0));
        self.harness.deliver_all(surface,[SurfaceEvent::ModifiersChanged(held),
            SurfaceEvent::Key{state:KeyState::Pressed,event:event.clone(),modifiers:held,timestamp},
            SurfaceEvent::Key{state:KeyState::Released,event,modifiers:held,timestamp},SurfaceEvent::ModifiersChanged(Modifiers::NONE)]);
        self.harness.pump();
        self.check()?;
        self.settle(false)?;
        Ok(json!({"key":key,"modifiers":modifiers,"burst":["modifiers","key.down","key.up","modifiers.clear"],"timeUs":timestamp.since_origin().as_micros()}))
    }

    pub fn text(&mut self,text:&str) -> Result<Value,Fault> {
        self.settle(true)?;
        let surface=self.window()?.surface().id();
        self.harness.deliver_all(surface,[SurfaceEvent::Ime(ImeEvent::Commit(text.into()))]);
        self.harness.pump();
        self.check()?;
        self.settle(false)?;
        Ok(json!({"burst":["native.ime.commit"],"utf8Bytes":text.len()}))
    }

    pub fn assert_node(&mut self,target:&Target,check:&Check) -> Result<Value,Fault> {
        self.settle(true)?;
        let last=self.probe.last.borrow();
        let last=last.as_ref().ok_or_else(stale)?;
        let index=selectors::unique(&last.tree,target,matches!(check,Check::Exists(false)))?;
        let node=index.map(|index|&last.tree.nodes[index]);
        // One node is serialized for the assertion record; its shape is unchanged.
        let record=node.map(|node|serde_json::to_value(node).expect("typed node serializes")).unwrap_or(Value::Null);
        let exists=json!(index.is_some());
        let (property,actual,truncated)=match check {
            Check::Exists(_)=>("exists",&exists,false),
            Check::Text(_)=>("text",&record["text"],record["textTruncated"]==true),
            Check::Role(_)=>("role",&record["role"],false),
            Check::Name(_)=>("name",&record["name"],record["nameTruncated"]==true),
            Check::Value(_)=>("value",&record["value"],record["valueTruncated"]==true),
            Check::Enabled(_)=>("enabled",&record["state"]["enabled"],false),
            Check::Focused(_)=>("focused",&record["state"]["focused"],false),
            Check::Visible(_)=>("visible",&record["state"]["visible"],false),
        };
        let (matches,expected)=match check {
            Check::Text(text)|Check::Role(text)|Check::Name(text)|Check::Value(text)=>
                (actual.as_str()==Some(text),selectors::short_string(text)),
            Check::Exists(value)|Check::Enabled(value)|Check::Focused(value)|Check::Visible(value)=>
                (actual.as_bool()==Some(*value),json!(value)),
        };
        let details=json!({"requested":selectors::requested(target),"property":property,"expected":expected,
            "actual":selectors::short(actual),"node":node.map(selectors::summary),"frameOrdinal":last.tree.frame_ordinal,"generation":last.tree.generation});
        if truncated || actual.is_null() {
            return Err(Error::new("E_ASSERTION_UNAVAILABLE",5,"the exact native property is unavailable or truncated").with_details(details).into());
        }
        if !matches { return Err(Error::new("E_ASSERTION",4,"native property differs from expected value").with_details(details).into()); }
        Ok(details)
    }
}

fn stale() -> Error { Error::new("E_TARGET_STALE",4,"target has no current coherent native snapshot") }
fn target_error(code:&'static str,message:&str,node:&crate::preview::evidence::NodeEvidence) -> Error {
    Error::new(code,4,message).with_details(json!({"target":selectors::summary(node),
        "state":serde_json::to_value(&node.state).expect("node state serializes"),
        "boundsDevice":serde_json::to_value(&node.bounds_device).expect("node bounds serialize")}))
}

/// Start with clipped native fragments, not a logical box with guessed scroll/transform offsets.
/// The 5x5 point search is conservative: a point is accepted only by actual native hit testing.
fn target_point(window:&zgui::runtime::Window,native:NodeId,snapshot:&OwnedFrame) -> Result<(DevicePoint,u32),Error> {
    let target=zgui_view_dom::id::to_document(native).ok_or_else(stale)?;
    if window.document().borrow().store().index_of(target).is_none() { return Err(stale()); }
    let layout=window.layout().borrow();
    let placements=window.placements();
    let extent=window.surface().size();
    let mut fragments=0;
    let mut samples=0;
    let mut nonempty=false;
    let mut blockers=Vec::new();
    for box_ in layout.boxes_of(target) {
        if layout.get(*box_).is_none_or(|node|node.style.get_inherited_box().visibility!=zgui_css::values::size::VisibilityValue::Visible) { continue; }
        for key in layout.fragments_of_box(*box_) {
            fragments+=1;
            if fragments>256 { return Err(Error::new("E_JOB_BUDGET",5,"target exceeds 256 native geometry fragments")); }
            let Some(fragment)=layout.fragment(*key) else { continue; };
            let bounds=zgui_layout::fragment::transform::placed::onto_device(fragment.border_box,fragment.transform,&placements);
            let clip=window.scene().clips.bounds_placed(fragment.clip,&|space|placements.get(space).copied());
            let left=bounds.origin.x.0.max(clip.origin.x.0).max(0.0);
            let top=bounds.origin.y.0.max(clip.origin.y.0).max(0.0);
            let right=(bounds.origin.x.0+bounds.size.width.0).min(clip.origin.x.0+clip.size.width.0).min(extent.width.0);
            let bottom=(bounds.origin.y.0+bounds.size.height.0).min(clip.origin.y.0+clip.size.height.0).min(extent.height.0);
            if ![left,top,right,bottom].iter().all(|value|value.is_finite()) || right<=left || bottom<=top { continue; }
            nonempty=true;
            for y in [0.5,0.1,0.9,0.25,0.75] { for x in [0.5,0.1,0.9,0.25,0.75] {
                let point=Point::new(DevicePx(left+(right-left)*x),DevicePx(top+(bottom-top)*y));
                samples+=1;
                let chain=window.chain_at(point);
                if chain.contains(&target) { return Ok((point,samples)); }
                if window.hit_contains(native,point) && let Some(blocker)=chain.last() {
                    let record=snapshot.native_nodes.iter().position(|node|zgui_view_dom::id::to_document(node.native)==Some(*blocker));
                    let summary=record.map(|index|selectors::summary(&snapshot.tree.nodes[index]))
                        .unwrap_or_else(||json!({"nativeIdentity":format!("{blocker:?}"),"ownership":"native"}));
                    if blockers.len()<3 && !blockers.contains(&summary) { blockers.push(summary); }
                }
            }}
        }
    }
    let (code,message)=if !nonempty { ("E_TARGET_CLIPPED","target fragments lie outside native clip/viewport bounds") }
        else if !blockers.is_empty() { ("E_TARGET_OBSCURED","native hit testing found blockers at the tested target points") }
        else { ("E_TARGET_UNREACHABLE","no tested clipped-fragment point reaches the target; inspect pointer-events and clip geometry") };
    Err(Error::new(code,4,message).with_details(json!({"pointSamples":samples,"fragmentCount":fragments,"blockers":blockers,"pointStrategy":"clipped-native-fragment-5x5"})))
}
