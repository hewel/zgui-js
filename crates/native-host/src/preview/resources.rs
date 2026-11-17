//! Package authority and capture readiness over the production native image loader.
use serde_json::{Value, json};
use crate::package::Package;
use super::Error;

pub(crate) fn configure(app: zgui::runtime::App, package: &Package) -> zgui::runtime::App {
    app.with_image_policy(zgui::runtime::ImagePolicy::package_only(package.image_assets()))
}

/// Pending allowances are exact current request IDs, never sources or stale generations.
/// A coherent placeholder may be captured without polling/publishing outstanding workers.
pub(crate) fn snapshot(window: &zgui::runtime::Window, allowed_pending: &[String]) -> Result<Value, Error> {
    let snapshot = window.image_resources();
    if let Some(error) = snapshot.graphics_failure {
        return Err(Error::new("E_GRAPHICS_UPLOAD", 7, error));
    }
    if snapshot.truncated {
        return Err(Error::new("E_ASSET_FAILED", 6, "native active resource evidence exceeded 4096 records"));
    }
    if !snapshot.failures.is_empty() {
        return Err(Error::new("E_ASSET_FAILED", 6, snapshot.failures.join("; ")));
    }
    let mut pending = Vec::new();
    let mut resources = Vec::with_capacity(snapshot.resources.len());
    for resource in snapshot.resources {
        use zgui::runtime::ImageState;
        if resource.state == ImageState::Failed && resource.visible {
            return Err(Error::new("E_ASSET_FAILED", 6, format!("{} ({}): {}", resource.id, resource.source, resource.failure.as_deref().unwrap_or("native image failed"))));
        }
        let state = match resource.state {
            ImageState::Requested => "requested",
            ImageState::Loading => "loading",
            ImageState::Ready => "ready-for-current-demand",
            ImageState::Failed => "failed",
            ImageState::Canceled => "canceled",
        };
        if resource.visible && matches!(resource.state, ImageState::Requested | ImageState::Loading) {
            pending.push(resource.id.clone());
        }
        resources.push(json!({
            "id": resource.id, "source": resource.source, "generation": resource.generation,
            "visible": resource.visible, "requestedVariant": resource.requested_variant,
            "attachedVariant": resource.attached_variant, "state": state, "failure": resource.failure,
            "uploaded": resource.uploaded,
        }));
    }
    for (index, allowed) in allowed_pending.iter().enumerate() {
        if allowed_pending[..index].contains(allowed) || !pending.contains(allowed) {
            return Err(Error::new("E_ASSET_FAILED", 6, format!("pending allowance does not identify a unique current visible pending image: {allowed}")));
        }
    }
    let ready = pending.iter().all(|id| allowed_pending.contains(id));
    Ok(json!({
        "requiredAssetsReady": ready,
        "pendingIds": pending,
        "allowedPendingAssets": allowed_pending,
        "resources": resources,
        "droppedRetiredResources": snapshot.dropped,
        "declaredAssets": snapshot.declared_assets,
        "droppedDeclaredAssets": snapshot.dropped_declarations,
        "publicationPolicy": "complete-batch-launch-order-v1",
        "resourceRevision": window.image_resource_revision(),
    }))
}
