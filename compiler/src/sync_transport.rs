// SPDX-License-Identifier: BUSL-1.1

use std::collections::VecDeque;

/// registry: cause = the sync transport did not receive a response within its deadline; remedy = retry the operation, since this condition is transient; a persistent timeout points at network latency or an unresponsive backend rather than the request
pub const E_RUNTIME_SYNC_TIMEOUT: &str = "E_RUNTIME_SYNC_TIMEOUT";
/// registry: cause = the sync transport link is down, or an update was requested while the backend is marked disconnected; remedy = restore connectivity and retry; a session may keep serving its committed configuration while disconnected, so this does not by itself invalidate local state
pub const E_RUNTIME_SYNC_TRANSPORT_DISCONNECTED: &str = "E_RUNTIME_SYNC_TRANSPORT_DISCONNECTED";
/// registry: cause = the transport received a payload it could not interpret, or a configuration or leaf hash field is not a 64-character hexadecimal digest; remedy = check that hash-shaped fields carry full digests, and that both ends of the link run compatible versions; unlike a timeout, this is not worth retrying unchanged
pub const E_RUNTIME_SYNC_PAYLOAD_INVALID: &str = "E_RUNTIME_SYNC_PAYLOAD_INVALID";
/// registry: cause = the sync transport failed for a reason it does not attribute to the link, the payload, or the deadline; remedy = report this with the transport configuration in use; retrying an unchanged request is not expected to help
pub const E_RUNTIME_SYNC_INTERNAL: &str = "E_RUNTIME_SYNC_INTERNAL";
/// registry: cause = a non-blocking publish was refused because the transport's outbound queue is already at its configured depth; remedy = let the queue drain and retry, or raise the configured queue depth if the publish rate is legitimately higher than the link can carry
pub const E_RUNTIME_SYNC_BACKPRESSURE: &str = "E_RUNTIME_SYNC_BACKPRESSURE";
/// registry: cause = the sync transport's credentials were rejected when connecting to the backend; remedy = check that the credentials are correct, current, and authorized for this device, then reconnect; retrying with the same rejected credentials will not succeed
pub const E_RUNTIME_SECURITY_AUTHN_FAILED: &str = "E_RUNTIME_SECURITY_AUTHN_FAILED";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MqttQosProfile {
    AtMostOnce,
    AtLeastOnce,
    ExactlyOnce,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub initial_backoff_ms: u64,
    pub max_backoff_ms: u64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_backoff_ms: 50,
            max_backoff_ms: 2_000,
        }
    }
}

impl RetryPolicy {
    pub fn backoff_for_attempt(&self, attempt: u32) -> u64 {
        let exponent = attempt.saturating_sub(1).min(16);
        let scaled = self.initial_backoff_ms.saturating_mul(1_u64 << exponent);
        scaled.min(self.max_backoff_ms)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncTransportError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

impl SyncTransportError {
    fn new(code: &str, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
            retryable,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MqttClientErrorKind {
    Disconnected,
    Timeout,
    Auth,
    InvalidPayload,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MqttClientError {
    pub kind: MqttClientErrorKind,
    pub message: String,
}

impl MqttClientError {
    pub fn disconnected(message: impl Into<String>) -> Self {
        Self {
            kind: MqttClientErrorKind::Disconnected,
            message: message.into(),
        }
    }

    pub fn timeout(message: impl Into<String>) -> Self {
        Self {
            kind: MqttClientErrorKind::Timeout,
            message: message.into(),
        }
    }

    pub fn auth(message: impl Into<String>) -> Self {
        Self {
            kind: MqttClientErrorKind::Auth,
            message: message.into(),
        }
    }

    pub fn invalid_payload(message: impl Into<String>) -> Self {
        Self {
            kind: MqttClientErrorKind::InvalidPayload,
            message: message.into(),
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            kind: MqttClientErrorKind::Internal,
            message: message.into(),
        }
    }
}

pub trait MqttClient {
    fn connect(&mut self) -> Result<(), MqttClientError>;
    fn publish(
        &mut self,
        topic: &str,
        qos: MqttQosProfile,
        payload: &[u8],
    ) -> Result<(), MqttClientError>;
    fn request(
        &mut self,
        topic: &str,
        qos: MqttQosProfile,
        payload: &[u8],
    ) -> Result<Vec<u8>, MqttClientError>;
}

pub trait SyncTransport {
    fn connect(&mut self) -> Result<(), SyncTransportError>;
    fn publish_nonblocking(&mut self, topic: &str, payload: &[u8]) -> Result<(), SyncTransportError>;
    fn request(&mut self, topic: &str, payload: &[u8]) -> Result<Vec<u8>, SyncTransportError>;
    fn drain_once(&mut self) -> Result<usize, SyncTransportError>;
    fn is_offline(&self) -> bool;
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingPublish {
    topic: String,
    payload: Vec<u8>,
}

pub struct MqttSyncTransport<C>
where
    C: MqttClient,
{
    client: C,
    retry_policy: RetryPolicy,
    qos_profile: MqttQosProfile,
    max_queue_depth: usize,
    pending: VecDeque<PendingPublish>,
    offline: bool,
    last_backoff_ms: u64,
}

impl<C> MqttSyncTransport<C>
where
    C: MqttClient,
{
    pub fn new(
        client: C,
        retry_policy: RetryPolicy,
        qos_profile: MqttQosProfile,
        max_queue_depth: usize,
    ) -> Self {
        Self {
            client,
            retry_policy,
            qos_profile,
            max_queue_depth: max_queue_depth.max(1),
            pending: VecDeque::new(),
            offline: true,
            last_backoff_ms: 0,
        }
    }

    pub fn queue_depth(&self) -> usize {
        self.pending.len()
    }

    pub fn last_backoff_ms(&self) -> u64 {
        self.last_backoff_ms
    }

    fn retry<T, F>(&mut self, mut op: F) -> Result<T, SyncTransportError>
    where
        F: FnMut(&mut C, MqttQosProfile) -> Result<T, MqttClientError>,
    {
        let attempts = self.retry_policy.max_attempts.max(1);
        for attempt in 0..attempts {
            match op(&mut self.client, self.qos_profile) {
                Ok(value) => {
                    self.offline = false;
                    self.last_backoff_ms = 0;
                    return Ok(value);
                }
                Err(error) => {
                    let mapped = map_mqtt_error(&error);
                    if !mapped.retryable || attempt + 1 >= attempts {
                        self.offline = mapped.retryable;
                        return Err(mapped);
                    }
                    self.offline = true;
                    self.last_backoff_ms = self.retry_policy.backoff_for_attempt(attempt + 1);
                }
            }
        }
        Err(SyncTransportError::new(
            E_RUNTIME_SYNC_INTERNAL,
            "retry loop terminated unexpectedly",
            false,
        ))
    }
}

impl<C> SyncTransport for MqttSyncTransport<C>
where
    C: MqttClient,
{
    fn connect(&mut self) -> Result<(), SyncTransportError> {
        self.retry(|client, _| client.connect())
    }

    fn publish_nonblocking(&mut self, topic: &str, payload: &[u8]) -> Result<(), SyncTransportError> {
        if self.pending.len() >= self.max_queue_depth {
            return Err(SyncTransportError::new(
                E_RUNTIME_SYNC_BACKPRESSURE,
                "publish queue is full",
                true,
            ));
        }
        self.pending.push_back(PendingPublish {
            topic: topic.to_string(),
            payload: payload.to_vec(),
        });
        Ok(())
    }

    fn request(&mut self, topic: &str, payload: &[u8]) -> Result<Vec<u8>, SyncTransportError> {
        self.retry(|client, qos| client.request(topic, qos, payload))
    }

    fn drain_once(&mut self) -> Result<usize, SyncTransportError> {
        let Some(message) = self.pending.front().cloned() else {
            return Ok(0);
        };

        self.retry(|client, qos| client.publish(&message.topic, qos, &message.payload))?;
        self.pending.pop_front();
        Ok(1)
    }

    fn is_offline(&self) -> bool {
        self.offline
    }
}

fn map_mqtt_error(error: &MqttClientError) -> SyncTransportError {
    match error.kind {
        MqttClientErrorKind::Disconnected => SyncTransportError::new(
            E_RUNTIME_SYNC_TRANSPORT_DISCONNECTED,
            error.message.clone(),
            true,
        ),
        MqttClientErrorKind::Timeout => {
            SyncTransportError::new(E_RUNTIME_SYNC_TIMEOUT, error.message.clone(), true)
        }
        MqttClientErrorKind::Auth => {
            SyncTransportError::new(E_RUNTIME_SECURITY_AUTHN_FAILED, error.message.clone(), false)
        }
        MqttClientErrorKind::InvalidPayload => {
            SyncTransportError::new(E_RUNTIME_SYNC_PAYLOAD_INVALID, error.message.clone(), false)
        }
        MqttClientErrorKind::Internal => {
            SyncTransportError::new(E_RUNTIME_SYNC_INTERNAL, error.message.clone(), false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct ScriptedMqttClient {
        connect_results: VecDeque<Result<(), MqttClientError>>,
        publish_results: VecDeque<Result<(), MqttClientError>>,
        request_results: VecDeque<Result<Vec<u8>, MqttClientError>>,
        connect_calls: usize,
        publish_calls: usize,
        request_calls: usize,
        publish_qos: Vec<MqttQosProfile>,
        request_qos: Vec<MqttQosProfile>,
    }

    impl ScriptedMqttClient {
        fn with_publish_results(results: Vec<Result<(), MqttClientError>>) -> Self {
            let mut client = Self::default();
            client.publish_results = results.into_iter().collect();
            client
        }

        fn with_connect_results(results: Vec<Result<(), MqttClientError>>) -> Self {
            let mut client = Self::default();
            client.connect_results = results.into_iter().collect();
            client
        }

        fn with_request_results(results: Vec<Result<Vec<u8>, MqttClientError>>) -> Self {
            let mut client = Self::default();
            client.request_results = results.into_iter().collect();
            client
        }
    }

    impl MqttClient for ScriptedMqttClient {
        fn connect(&mut self) -> Result<(), MqttClientError> {
            self.connect_calls += 1;
            self.connect_results.pop_front().unwrap_or(Ok(()))
        }

        fn publish(
            &mut self,
            _topic: &str,
            qos: MqttQosProfile,
            _payload: &[u8],
        ) -> Result<(), MqttClientError> {
            self.publish_calls += 1;
            self.publish_qos.push(qos);
            self.publish_results.pop_front().unwrap_or(Ok(()))
        }

        fn request(
            &mut self,
            _topic: &str,
            qos: MqttQosProfile,
            _payload: &[u8],
        ) -> Result<Vec<u8>, MqttClientError> {
            self.request_calls += 1;
            self.request_qos.push(qos);
            self.request_results
                .pop_front()
                .unwrap_or_else(|| Ok(Vec::new()))
        }
    }

    #[test]
    fn mqtt_transport_retries_transient_publish_disconnect() {
        let client = ScriptedMqttClient::with_publish_results(vec![
            Err(MqttClientError::disconnected("link down")),
            Ok(()),
        ]);
        let mut transport = MqttSyncTransport::new(
            client,
            RetryPolicy {
                max_attempts: 2,
                initial_backoff_ms: 10,
                max_backoff_ms: 100,
            },
            MqttQosProfile::AtLeastOnce,
            8,
        );

        transport
            .publish_nonblocking("cfg/test", b"payload")
            .expect("queue publish");
        let drained = transport.drain_once().expect("drain once");
        assert_eq!(drained, 1);
        assert_eq!(transport.queue_depth(), 0);
        assert!(!transport.is_offline());
        assert_eq!(transport.client.publish_calls, 2);
        assert_eq!(
            transport.client.publish_qos,
            vec![MqttQosProfile::AtLeastOnce, MqttQosProfile::AtLeastOnce]
        );
    }

    #[test]
    fn mqtt_transport_maps_timeout_error_deterministically() {
        let client = ScriptedMqttClient::with_request_results(vec![Err(MqttClientError::timeout(
            "request timed out",
        ))]);
        let mut transport = MqttSyncTransport::new(
            client,
            RetryPolicy {
                max_attempts: 1,
                initial_backoff_ms: 25,
                max_backoff_ms: 50,
            },
            MqttQosProfile::AtMostOnce,
            8,
        );

        let error = transport
            .request("cfg/test/request", b"payload")
            .expect_err("timeout expected");
        assert_eq!(error.code, E_RUNTIME_SYNC_TIMEOUT);
        assert!(error.retryable);
    }

    #[test]
    fn mqtt_transport_auth_failure_is_not_retryable() {
        let client = ScriptedMqttClient::with_connect_results(vec![Err(MqttClientError::auth(
            "credentials rejected",
        ))]);
        let mut transport = MqttSyncTransport::new(
            client,
            RetryPolicy {
                max_attempts: 3,
                initial_backoff_ms: 10,
                max_backoff_ms: 20,
            },
            MqttQosProfile::ExactlyOnce,
            4,
        );

        let error = transport.connect().expect_err("auth failure expected");
        assert_eq!(error.code, E_RUNTIME_SECURITY_AUTHN_FAILED);
        assert!(!error.retryable);
        assert_eq!(transport.client.connect_calls, 1);
    }

    #[test]
    fn mqtt_transport_queue_backpressure_is_bounded() {
        let client = ScriptedMqttClient::default();
        let mut transport = MqttSyncTransport::new(
            client,
            RetryPolicy::default(),
            MqttQosProfile::AtLeastOnce,
            1,
        );

        transport
            .publish_nonblocking("cfg/test", b"a")
            .expect("first publish should queue");
        let error = transport
            .publish_nonblocking("cfg/test", b"b")
            .expect_err("second publish should backpressure");
        assert_eq!(error.code, E_RUNTIME_SYNC_BACKPRESSURE);
        assert!(error.retryable);
    }

    #[test]
    fn mqtt_transport_keeps_queued_publish_when_disconnect_persists() {
        let client = ScriptedMqttClient::with_publish_results(vec![Err(
            MqttClientError::disconnected("still down"),
        )]);
        let mut transport = MqttSyncTransport::new(
            client,
            RetryPolicy {
                max_attempts: 1,
                initial_backoff_ms: 10,
                max_backoff_ms: 20,
            },
            MqttQosProfile::AtLeastOnce,
            4,
        );

        transport
            .publish_nonblocking("cfg/test", b"msg")
            .expect("publish queued");
        let error = transport.drain_once().expect_err("drain should fail");
        assert_eq!(error.code, E_RUNTIME_SYNC_TRANSPORT_DISCONNECTED);
        assert_eq!(transport.queue_depth(), 1);
        assert!(transport.is_offline());
    }
}
