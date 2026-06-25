use std::path::Path;

use tracing::warn;

use crate::constants::RESERVED_ROUTE_PREFIXES;
use crate::error::ExtensionError;
use crate::types::{ExtWebui, WebuiContribution};

/// Validate that a WebUI route path is within the extension's namespace
/// and does not use reserved prefixes.
fn validate_route(route_path: &str, extension_name: &str) -> Result<(), ExtensionError> {
    let expected_prefix = format!("/{extension_name}/");

    if !route_path.starts_with(&expected_prefix) {
        return Err(ExtensionError::InvalidWebuiRouteNamespace {
            extension_name: extension_name.to_owned(),
            route: route_path.to_owned(),
        });
    }

    for prefix in RESERVED_ROUTE_PREFIXES {
        if route_path.starts_with(prefix) {
            return Err(ExtensionError::ReservedWebuiRoute {
                route: route_path.to_owned(),
                prefix: (*prefix).to_owned(),
            });
        }
    }

    Ok(())
}

/// Resolve a single WebUI contribution.
///
/// All routes are validated to be within the `/{extensionName}/` namespace
/// and not using reserved prefixes.
pub fn resolve_webui(
    webui: &ExtWebui,
    extension_name: &str,
    ext_dir: &Path,
) -> Result<WebuiContribution, ExtensionError> {
    for route in &webui.routes {
        validate_route(&route.path, extension_name)?;
    }

    let directory = ext_dir.join(&webui.directory).to_string_lossy().into_owned();

    Ok(WebuiContribution {
        extension_name: extension_name.to_owned(),
        id: webui.id.clone(),
        directory,
        routes: webui.routes.clone(),
    })
}

/// Resolve all WebUI contributions from an extension.
pub fn resolve_webui_contributions(
    webuis: &[ExtWebui],
    extension_name: &str,
    ext_dir: &Path,
) -> Vec<WebuiContribution> {
    webuis
        .iter()
        .filter_map(|w| {
            resolve_webui(w, extension_name, ext_dir)
                .map_err(|e| {
                    warn!(
                        extension = extension_name,
                        webui_id = w.id,
                        "Failed to resolve WebUI: {e}"
                    );
                    e
                })
                .ok()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ExtWebuiRoute;

    fn make_route(path: &str) -> ExtWebuiRoute {
        ExtWebuiRoute {
            path: path.into(),
            method: "GET".into(),
            handler: "handler.js".into(),
        }
    }

    #[test]
    fn test_validate_route_valid_namespace() {
        assert!(validate_route("/my-ext/api/data", "my-ext").is_ok());
        assert!(validate_route("/my-ext/page", "my-ext").is_ok());
    }

    #[test]
    fn test_validate_route_wrong_namespace() {
        let err = validate_route("/other-ext/api", "my-ext").unwrap_err();
        assert!(matches!(err, ExtensionError::InvalidWebuiRouteNamespace { .. }));
    }

    #[test]
    fn test_validate_route_reserved_prefix() {
        let err = validate_route("/api/extensions", "api").unwrap_err();
        assert!(matches!(err, ExtensionError::ReservedWebuiRoute { .. }));
    }

    #[test]
    fn test_resolve_webui_valid() {
        let webui = ExtWebui {
            id: "web-1".into(),
            directory: "dist".into(),
            routes: vec![make_route("/my-ext/dashboard")],
        };

        let result = resolve_webui(&webui, "my-ext", Path::new("/ext/my-ext")).unwrap();

        assert_eq!(result.extension_name, "my-ext");
        assert_eq!(result.id, "web-1");
        assert!(result.directory.contains("dist"));
        assert_eq!(result.routes.len(), 1);
    }

    #[test]
    fn test_resolve_webui_invalid_route_rejected() {
        let webui = ExtWebui {
            id: "web-bad".into(),
            directory: "dist".into(),
            routes: vec![make_route("/other-ext/api")],
        };

        let err = resolve_webui(&webui, "my-ext", Path::new("/ext/my-ext")).unwrap_err();
        assert!(matches!(err, ExtensionError::InvalidWebuiRouteNamespace { .. }));
    }

    #[test]
    fn test_resolve_webui_no_routes() {
        let webui = ExtWebui {
            id: "static-only".into(),
            directory: "public".into(),
            routes: vec![],
        };

        let result = resolve_webui(&webui, "my-ext", Path::new("/ext/my-ext")).unwrap();
        assert!(result.routes.is_empty());
    }

    #[test]
    fn test_resolve_webui_contributions_filters_invalid() {
        let webuis = vec![
            ExtWebui {
                id: "good".into(),
                directory: "dist".into(),
                routes: vec![make_route("/my-ext/page")],
            },
            ExtWebui {
                id: "bad".into(),
                directory: "dist".into(),
                routes: vec![make_route("/other/page")],
            },
        ];

        let result = resolve_webui_contributions(&webuis, "my-ext", Path::new("/ext/my-ext"));
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, "good");
    }

    #[test]
    fn test_validate_route_ws_reserved() {
        let err = validate_route("/ws/stream", "ws").unwrap_err();
        assert!(matches!(err, ExtensionError::ReservedWebuiRoute { .. }));
    }

    #[test]
    fn test_validate_route_auth_reserved() {
        let err = validate_route("/auth/login", "auth").unwrap_err();
        assert!(matches!(err, ExtensionError::ReservedWebuiRoute { .. }));
    }
}
