use nomifun_api_types::FetchModelsResponse;
use nomifun_common::AppError;
use nomifun_model_invoke::root_candidates;
use tracing::debug;

use super::FetchConfig;
use super::fetchers::fetch_openai_compatible_with_auth;

/// How conclusively one candidate root answered.
///
/// The distinction that matters is between "this URL is wrong" and "this URL is
/// right but the key is not". A 401/403 proves an endpoint exists and is
/// enforcing auth, so it identifies the correct root even when the credential
/// is dead, expired, or out of quota — the case where auto-fix used to give up
/// entirely and report nothing useful.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum CandidateOutcome {
    /// The root answered with a usable model catalog.
    Listed,
    /// The root exists and rejected the credential.
    CredentialsRejected,
}

/// Try candidate roots and return the first conclusive one.
///
/// Candidates are probed concurrently but resolved in *list* order, so the
/// answer does not depend on which host replied first. A catalog listing beats a
/// credential rejection; among equals, the earlier candidate wins.
pub(crate) async fn try_fix_url(
    client: &reqwest::Client,
    config: &FetchConfig,
) -> Result<FetchModelsResponse, AppError> {
    let candidates = root_candidates(&config.base_url);

    debug!(
        base_url = %config.base_url,
        candidate_count = candidates.len(),
        "Starting URL auto-fix probe"
    );

    let probes = candidates.iter().map(|candidate| {
        let client = client.clone();
        let auth = config.auth.clone();
        let candidate = candidate.clone();
        async move {
            match fetch_openai_compatible_with_auth(&client, &candidate, &auth).await {
                Ok(models) => Some((CandidateOutcome::Listed, models)),
                // A rejected credential still confirms the endpoint. Carry no
                // models: the catalog is unknown until the key works.
                Err(AppError::Unauthorized(_)) | Err(AppError::Forbidden(_)) => {
                    Some((CandidateOutcome::CredentialsRejected, Vec::new()))
                }
                Err(_) => None,
            }
        }
    });
    let outcomes = futures::future::join_all(probes).await;

    let best = candidates
        .iter()
        .zip(outcomes)
        .filter_map(|(candidate, outcome)| {
            outcome.map(|(rank, models)| (rank, candidate.clone(), models))
        })
        .min_by_key(|(rank, _, _)| *rank);

    let Some((rank, fixed_url, models)) = best else {
        return Err(AppError::BadGateway(
            "All URL variants failed during auto-fix".into(),
        ));
    };

    debug!(fixed_url = %fixed_url, ?rank, "URL auto-fix resolved a root");

    if rank == CandidateOutcome::CredentialsRejected {
        // Reporting the corrected root here would look like success while the
        // catalog is still empty. The caller's error already says the key was
        // rejected; the provider-level probe is what offers the root as an
        // adoptable suggestion.
        return Err(AppError::Unauthorized(format!(
            "Remote API rejected the API key at {fixed_url}"
        )));
    }

    Ok(FetchModelsResponse {
        models,
        fixed_base_url: Some(fixed_url),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidates_come_from_the_shared_algebra_and_lead_with_the_configured_root() {
        let candidates = root_candidates("https://api.example.com");
        assert_eq!(candidates[0], "https://api.example.com");
        assert!(candidates.contains(&"https://api.example.com/v1".to_string()));
        assert!(candidates.contains(&"https://api.example.com/api/v1".to_string()));
        assert!(candidates.contains(&"https://api.example.com/openai/v1".to_string()));
    }

    #[test]
    fn candidates_never_double_a_version_the_user_already_typed() {
        for candidate in root_candidates("https://api.example.com/v1") {
            assert!(
                !candidate.contains("/v1/v1"),
                "candidate must not double a version: {candidate}"
            );
        }
        // The bare root must be probed so a correctly configured versioned root
        // can still be confirmed.
        assert!(
            root_candidates("https://api.example.com/v1")
                .contains(&"https://api.example.com".to_string())
        );
    }

    #[test]
    fn candidates_have_no_double_slash_after_the_scheme() {
        for candidate in root_candidates("https://api.example.com") {
            let after_scheme = candidate.strip_prefix("https://").unwrap();
            assert!(!after_scheme.contains("//"), "Double slash found in: {candidate}");
        }
    }

    #[test]
    fn a_listed_catalog_outranks_a_credential_rejection() {
        assert!(CandidateOutcome::Listed < CandidateOutcome::CredentialsRejected);
    }
}
