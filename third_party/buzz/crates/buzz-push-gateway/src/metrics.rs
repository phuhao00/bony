//! Sanitized, bounded-cardinality metrics for the push gateway, recorded via
//! the `metrics` facade (`metrics::counter!`, `metrics::histogram!`).
//!
//! No exporter or scrape endpoint is installed — this deployment target runs
//! single-instance without external monitoring infrastructure. Recording
//! through the facade is a harmless no-op when no recorder is installed.
//!
//! Every label value emitted here is a compile-time `&'static str` drawn from a
//! closed set (the [`DeliveryOutcome`] variants, the gateway's fixed error
//! codes, and the handler stages). No endpoint, device token, relay pubkey,
//! request id, or any other request-scoped identifier is ever used as a label,
//! so metric cardinality is structurally bounded regardless of traffic.

use crate::apns::DeliveryOutcome;

/// Stable metric label for each sanitized delivery outcome. The mapping is total
/// over the closed [`DeliveryOutcome`] enum, so the `outcome` label can only take
/// these six values.
fn outcome_label(outcome: DeliveryOutcome) -> &'static str {
    match outcome {
        DeliveryOutcome::Accepted => "accepted",
        DeliveryOutcome::InvalidEndpoint { .. } => "invalid_endpoint",
        DeliveryOutcome::Retry { .. } => "retry",
        DeliveryOutcome::RefreshCredential => "refresh_credential",
        DeliveryOutcome::ConfigurationFault => "configuration_fault",
        DeliveryOutcome::PermanentRequestFault => "permanent_request_fault",
    }
}

/// Record the terminal APNs outcome and its send round-trip latency.
pub fn record_apns_delivery(outcome: DeliveryOutcome, seconds: f64) {
    metrics::counter!("push_gateway_apns_deliveries_total", "outcome" => outcome_label(outcome))
        .increment(1);
    metrics::histogram!("push_gateway_apns_delivery_seconds").record(seconds);
}

/// Record that a cached provider credential was refreshed after APNs reported expiry.
pub fn record_credential_refresh() {
    metrics::counter!("push_gateway_apns_credential_refreshes_total").increment(1);
}

/// Delivery-admission result at the `authorize_delivery` seam.
#[derive(Debug, Clone, Copy)]
pub enum Admission {
    /// A delivery permit was issued.
    Admitted,
    /// The replay/quota/authority fence rejected the request.
    Rejected,
    /// The authority store was transiently unavailable.
    Unavailable,
}

/// Record the outcome of a delivery-admission attempt.
pub fn record_admission(result: Admission) {
    let label = match result {
        Admission::Admitted => "admitted",
        Admission::Rejected => "rejected",
        Admission::Unavailable => "unavailable",
    };
    metrics::counter!("push_gateway_admissions_total", "result" => label).increment(1);
}

/// Record a delivery-path error, tagged by the static failure class. This
/// counter covers only the `/v1/deliveries/apns` handler's post-admission exit
/// classes (admission rejection/unavailability, profile mismatch, token-custody
/// open failure, and detached finish/join failure); pre-admission request/auth/
/// attestation validation on the enrollment and delegation handlers is not
/// counted here. `class` is always a compile-time constant.
pub fn record_delivery_error(class: &'static str) {
    metrics::counter!("push_gateway_delivery_errors_total", "class" => class).increment(1);
}

/// Record a retention-reaper sweep failure.
pub fn record_reaper_failure() {
    metrics::counter!("push_gateway_reaper_failures_total").increment(1);
}

/// Why a readiness probe reported not-ready.
#[derive(Debug, Clone, Copy)]
pub enum ReadinessFailure {
    /// The process is draining and no longer accepting traffic.
    NotAccepting,
    /// The authority store readiness check failed.
    Authority,
}

/// Record a readiness-probe failure by cause.
pub fn record_readiness_failure(cause: ReadinessFailure) {
    let label = match cause {
        ReadinessFailure::NotAccepting => "not_accepting",
        ReadinessFailure::Authority => "authority",
    };
    metrics::counter!("push_gateway_readiness_failures_total", "cause" => label).increment(1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_label_covers_every_variant_with_static_strings() {
        // Exhaustive over the closed enum; each arm is a compile-time constant,
        // so the `outcome` label is structurally bounded to these six values.
        for (outcome, expected) in [
            (DeliveryOutcome::Accepted, "accepted"),
            (
                DeliveryOutcome::InvalidEndpoint {
                    unregistered_at: Some(7),
                },
                "invalid_endpoint",
            ),
            (
                DeliveryOutcome::Retry {
                    retry_after_seconds: Some(30),
                },
                "retry",
            ),
            (DeliveryOutcome::RefreshCredential, "refresh_credential"),
            (DeliveryOutcome::ConfigurationFault, "configuration_fault"),
            (
                DeliveryOutcome::PermanentRequestFault,
                "permanent_request_fault",
            ),
        ] {
            assert_eq!(outcome_label(outcome), expected);
        }
    }

    // No global recorder is installed in this deployment target, so these
    // calls only exercise that the facade macros compile and run without a
    // recorder attached (a documented no-op behavior of the `metrics` crate).
    #[test]
    fn record_helpers_run_without_a_recorder_installed() {
        record_apns_delivery(DeliveryOutcome::Accepted, 0.012);
        record_apns_delivery(
            DeliveryOutcome::InvalidEndpoint {
                unregistered_at: None,
            },
            0.030,
        );
        record_credential_refresh();
        record_admission(Admission::Admitted);
        record_admission(Admission::Rejected);
        record_admission(Admission::Unavailable);
        record_delivery_error("invalid_grant");
        record_delivery_error("finish_failed");
        record_reaper_failure();
        record_readiness_failure(ReadinessFailure::NotAccepting);
        record_readiness_failure(ReadinessFailure::Authority);
    }
}
