//! The e2e's stand-in for GitHub's releases API. Only an instrumented build has it: `cargo llvm-cov` sets
//! `cfg(coverage)`, a release build never does, so no release binary reads [`VARIABLE`]. The conformance suite's
//! coverage runs point [`Source::github`] at a local server with it, and the server sees the check and its rate-limit
//! retry with the suite's fake token, as GitHub would.
use std::time::Duration;

use crate::Source;
use crate::download::DOWNLOADS;

/// The stand-in's API URL, for `http://127.0.0.1` too.
pub(crate) const VARIABLE: &str = "NGTERM_TEST_RELEASES_API";

fn api() -> Option<String> {
    std::env::var(VARIABLE).ok().filter(|api| !api.is_empty())
}

pub(crate) fn source() -> Option<Source> {
    Some(Source {
        api: api()?,
        downloads: DOWNLOADS.into(),
        agent: ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .build()
            .into(),
    })
}

/// Whether `api` is the stand-in's, which gets the local token as GitHub's API does.
pub(crate) fn is_api(api: &str) -> bool {
    self::api().is_some_and(|stand_in| stand_in == api)
}
