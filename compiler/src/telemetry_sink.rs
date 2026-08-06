// SPDX-License-Identifier: BUSL-1.1

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

/// registry: cause = a telemetry spool or archive file could not be created, read, written, renamed, or removed on the local device; remedy = check the free space, permissions, and mount state of the telemetry directory; the diagnostic names the specific file and the underlying system error
pub const E_RUNTIME_TELEMETRY_IO: &str = "E_RUNTIME_TELEMETRY_IO";
/// registry: cause = reserved for a telemetry sink refusing a record because its buffer is full; no current code path emits it, because the sinks drop the oldest record and count the drop instead; remedy = no action is needed for this code; to detect record loss, read the dropped-record counter a sink exposes rather than watching for this diagnostic
pub const E_RUNTIME_TELEMETRY_BACKPRESSURE: &str = "E_RUNTIME_TELEMETRY_BACKPRESSURE";
/// registry: cause = a telemetry flush reached its publisher, and the publisher itself rejected or could not deliver the batch; remedy = treat this as a delivery failure rather than a data failure: the records stay spooled, so restoring the publisher's availability lets the next flush drain them
pub const E_RUNTIME_TELEMETRY_PUBLISH_FAILED: &str = "E_RUNTIME_TELEMETRY_PUBLISH_FAILED";
/// registry: cause = a telemetry record could not be formed or read back: an audit event with a blank actor, or a spooled record that will not serialize or parse; remedy = supply a non-empty actor on every audit event; a record that fails to parse on read-back indicates a damaged spool file, which can be removed to resume telemetry
pub const E_RUNTIME_TELEMETRY_RECORD_CORRUPT: &str = "E_RUNTIME_TELEMETRY_RECORD_CORRUPT";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelemetrySinkError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

impl TelemetrySinkError {
    pub fn new(code: &str, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
            retryable,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SinkRecordKind {
    Telemetry,
    Audit,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SinkRecord {
    pub sequence: u64,
    pub timestamp_unix_ms: u64,
    pub topic: String,
    pub kind: SinkRecordKind,
    pub payload: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub committed_configuration_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub working_configuration_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TelemetryBatch {
    pub records: Vec<SinkRecord>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuditEvent {
    pub sequence: u64,
    pub timestamp_unix_ms: u64,
    pub topic: String,
    pub payload: serde_json::Value,
    pub actor: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub committed_configuration_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub working_configuration_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlushReport {
    pub attempted: usize,
    pub published: usize,
    pub remaining: usize,
    pub dropped: u64,
}

pub trait TelemetryPublisher {
    fn publish(&mut self, records: &[SinkRecord]) -> Result<(), TelemetrySinkError>;
}

pub trait TelemetrySink {
    fn emit_batch(&mut self, batch: TelemetryBatch) -> Result<(), TelemetrySinkError>;
    fn flush(&mut self) -> Result<FlushReport, TelemetrySinkError>;
    fn replay_from(
        &self,
        sequence_exclusive: u64,
        limit: usize,
    ) -> Result<Vec<SinkRecord>, TelemetrySinkError>;
}

pub trait AuditSink {
    fn emit_event(&mut self, event: AuditEvent) -> Result<(), TelemetrySinkError>;
}

pub struct BufferedPushSink<P>
where
    P: TelemetryPublisher,
{
    publisher: P,
    queue: VecDeque<SinkRecord>,
    max_records: usize,
    publish_batch_size: usize,
    dropped_records: u64,
}

impl<P> BufferedPushSink<P>
where
    P: TelemetryPublisher,
{
    pub fn new(publisher: P, max_records: usize, publish_batch_size: usize) -> Self {
        Self {
            publisher,
            queue: VecDeque::new(),
            max_records: max_records.max(1),
            publish_batch_size: publish_batch_size.max(1),
            dropped_records: 0,
        }
    }

    pub fn dropped_records(&self) -> u64 {
        self.dropped_records
    }

    fn enqueue_records(&mut self, mut records: Vec<SinkRecord>) {
        for record in &mut records {
            sanitize_record(record);
        }
        self.queue.extend(records);
        while self.queue.len() > self.max_records {
            self.queue.pop_front();
            self.dropped_records = self.dropped_records.saturating_add(1);
        }
    }
}

impl<P> TelemetrySink for BufferedPushSink<P>
where
    P: TelemetryPublisher,
{
    fn emit_batch(&mut self, mut batch: TelemetryBatch) -> Result<(), TelemetrySinkError> {
        for record in &mut batch.records {
            record.kind = SinkRecordKind::Telemetry;
        }
        self.enqueue_records(batch.records);
        Ok(())
    }

    fn flush(&mut self) -> Result<FlushReport, TelemetrySinkError> {
        let attempted = self.queue.len();
        let mut published = 0_usize;

        while !self.queue.is_empty() {
            let chunk_size = self.publish_batch_size.min(self.queue.len());
            let chunk: Vec<SinkRecord> = self.queue.iter().take(chunk_size).cloned().collect();
            self.publisher.publish(&chunk).map_err(|error| {
                TelemetrySinkError::new(
                    &error.code,
                    format!("Buffered push publish failed: {}", error.message),
                    error.retryable,
                )
            })?;
            self.queue.drain(0..chunk_size);
            published += chunk_size;
        }

        Ok(FlushReport {
            attempted,
            published,
            remaining: self.queue.len(),
            dropped: self.dropped_records,
        })
    }

    fn replay_from(
        &self,
        sequence_exclusive: u64,
        limit: usize,
    ) -> Result<Vec<SinkRecord>, TelemetrySinkError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let mut records: Vec<SinkRecord> = self
            .queue
            .iter()
            .filter(|record| record.sequence > sequence_exclusive)
            .cloned()
            .collect();
        records.sort_by_key(|record| record.sequence);
        records.truncate(limit);
        Ok(records)
    }
}

impl<P> AuditSink for BufferedPushSink<P>
where
    P: TelemetryPublisher,
{
    fn emit_event(&mut self, event: AuditEvent) -> Result<(), TelemetrySinkError> {
        let record = audit_event_to_record(event)?;
        self.enqueue_records(vec![record]);
        Ok(())
    }
}

pub struct DeferredUploadSink<P>
where
    P: TelemetryPublisher,
{
    publisher: P,
    spool_path: PathBuf,
    queue: VecDeque<SinkRecord>,
    max_records: usize,
    max_spool_bytes: u64,
    publish_batch_size: usize,
    dropped_records: u64,
}

impl<P> DeferredUploadSink<P>
where
    P: TelemetryPublisher,
{
    pub fn open(
        spool_path: impl Into<PathBuf>,
        publisher: P,
        max_records: usize,
        max_spool_bytes: u64,
        publish_batch_size: usize,
    ) -> Result<Self, TelemetrySinkError> {
        let spool_path = spool_path.into();
        if let Some(parent) = spool_path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                TelemetrySinkError::new(
                    E_RUNTIME_TELEMETRY_IO,
                    format!(
                        "Failed to create deferred spool directory '{}': {}",
                        parent.display(),
                        error
                    ),
                    true,
                )
            })?;
        }
        let queue = load_spool_queue(&spool_path)?;
        let mut sink = Self {
            publisher,
            spool_path,
            queue,
            max_records: max_records.max(1),
            max_spool_bytes: max_spool_bytes.max(1),
            publish_batch_size: publish_batch_size.max(1),
            dropped_records: 0,
        };
        sink.enforce_bounds();
        sink.persist_queue()?;
        Ok(sink)
    }

    pub fn dropped_records(&self) -> u64 {
        self.dropped_records
    }

    fn enqueue_records(&mut self, mut records: Vec<SinkRecord>) {
        for record in &mut records {
            sanitize_record(record);
        }
        self.queue.extend(records);
        self.enforce_bounds();
    }

    fn enforce_bounds(&mut self) {
        while self.queue.len() > self.max_records {
            self.queue.pop_front();
            self.dropped_records = self.dropped_records.saturating_add(1);
        }
        while queue_serialized_bytes(&self.queue) > self.max_spool_bytes && !self.queue.is_empty() {
            self.queue.pop_front();
            self.dropped_records = self.dropped_records.saturating_add(1);
        }
    }

    fn persist_queue(&self) -> Result<(), TelemetrySinkError> {
        persist_spool_queue(&self.spool_path, &self.queue)
    }
}

impl<P> TelemetrySink for DeferredUploadSink<P>
where
    P: TelemetryPublisher,
{
    fn emit_batch(&mut self, mut batch: TelemetryBatch) -> Result<(), TelemetrySinkError> {
        for record in &mut batch.records {
            record.kind = SinkRecordKind::Telemetry;
        }
        self.enqueue_records(batch.records);
        self.persist_queue()
    }

    fn flush(&mut self) -> Result<FlushReport, TelemetrySinkError> {
        let attempted = self.queue.len();
        let mut published = 0_usize;

        while !self.queue.is_empty() {
            let chunk_size = self.publish_batch_size.min(self.queue.len());
            let chunk: Vec<SinkRecord> = self.queue.iter().take(chunk_size).cloned().collect();
            self.publisher.publish(&chunk).map_err(|error| {
                TelemetrySinkError::new(
                    &error.code,
                    format!("Deferred upload publish failed: {}", error.message),
                    error.retryable,
                )
            })?;
            self.queue.drain(0..chunk_size);
            published += chunk_size;
            self.persist_queue()?;
        }

        Ok(FlushReport {
            attempted,
            published,
            remaining: self.queue.len(),
            dropped: self.dropped_records,
        })
    }

    fn replay_from(
        &self,
        sequence_exclusive: u64,
        limit: usize,
    ) -> Result<Vec<SinkRecord>, TelemetrySinkError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let mut records: Vec<SinkRecord> = self
            .queue
            .iter()
            .filter(|record| record.sequence > sequence_exclusive)
            .cloned()
            .collect();
        records.sort_by_key(|record| record.sequence);
        records.truncate(limit);
        Ok(records)
    }
}

impl<P> AuditSink for DeferredUploadSink<P>
where
    P: TelemetryPublisher,
{
    fn emit_event(&mut self, event: AuditEvent) -> Result<(), TelemetrySinkError> {
        let record = audit_event_to_record(event)?;
        self.enqueue_records(vec![record]);
        self.persist_queue()
    }
}

#[derive(Debug, Clone)]
struct ArchiveChunk {
    path: PathBuf,
    min_sequence: u64,
    max_sequence: u64,
    file_counter: u64,
    size_bytes: u64,
    record_count: usize,
}

pub struct DiskArchiveSink {
    archive_dir: PathBuf,
    max_archive_files: usize,
    max_archive_bytes: u64,
    chunks: VecDeque<ArchiveChunk>,
    next_file_counter: u64,
    dropped_records: u64,
}

impl DiskArchiveSink {
    pub fn open(
        archive_dir: impl Into<PathBuf>,
        max_archive_files: usize,
        max_archive_bytes: u64,
    ) -> Result<Self, TelemetrySinkError> {
        let archive_dir = archive_dir.into();
        fs::create_dir_all(&archive_dir).map_err(|error| {
            TelemetrySinkError::new(
                E_RUNTIME_TELEMETRY_IO,
                format!(
                    "Failed to create archive directory '{}': {}",
                    archive_dir.display(),
                    error
                ),
                true,
            )
        })?;

        let chunks = load_archive_chunks(&archive_dir)?;
        let next_file_counter = chunks
            .iter()
            .map(|chunk| chunk.file_counter)
            .max()
            .unwrap_or(0)
            .saturating_add(1);

        let mut sink = Self {
            archive_dir,
            max_archive_files: max_archive_files.max(1),
            max_archive_bytes: max_archive_bytes.max(1),
            chunks,
            next_file_counter,
            dropped_records: 0,
        };
        sink.enforce_bounds()?;
        Ok(sink)
    }

    pub fn dropped_records(&self) -> u64 {
        self.dropped_records
    }

    fn enqueue_records(&mut self, mut records: Vec<SinkRecord>) -> Result<(), TelemetrySinkError> {
        if records.is_empty() {
            return Ok(());
        }
        for record in &mut records {
            sanitize_record(record);
        }
        records.sort_by_key(|record| record.sequence);
        let min_sequence = records
            .first()
            .map(|record| record.sequence)
            .unwrap_or_default();
        let max_sequence = records
            .last()
            .map(|record| record.sequence)
            .unwrap_or_default();
        let file_counter = self.next_file_counter;
        self.next_file_counter = self.next_file_counter.saturating_add(1);
        let file_name = format!(
            "chunk-{min_sequence:020}-{max_sequence:020}-{file_counter:020}.jsonl.gz"
        );
        let path = self.archive_dir.join(file_name);
        write_archive_chunk(&path, &records)?;
        let size_bytes = fs::metadata(&path)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        self.chunks.push_back(ArchiveChunk {
            path,
            min_sequence,
            max_sequence,
            file_counter,
            size_bytes,
            record_count: records.len(),
        });
        self.enforce_bounds()
    }

    fn enforce_bounds(&mut self) -> Result<(), TelemetrySinkError> {
        while self.chunks.len() > self.max_archive_files || self.total_archive_bytes() > self.max_archive_bytes {
            let Some(chunk) = self.chunks.pop_front() else {
                break;
            };
            fs::remove_file(&chunk.path).map_err(|error| {
                TelemetrySinkError::new(
                    E_RUNTIME_TELEMETRY_IO,
                    format!(
                        "Failed to prune archive chunk '{}': {}",
                        chunk.path.display(),
                        error
                    ),
                    true,
                )
            })?;
            self.dropped_records = self
                .dropped_records
                .saturating_add(chunk.record_count as u64);
        }
        Ok(())
    }

    fn total_archive_bytes(&self) -> u64 {
        self.chunks
            .iter()
            .fold(0_u64, |acc, chunk| acc.saturating_add(chunk.size_bytes))
    }
}

impl TelemetrySink for DiskArchiveSink {
    fn emit_batch(&mut self, mut batch: TelemetryBatch) -> Result<(), TelemetrySinkError> {
        for record in &mut batch.records {
            record.kind = SinkRecordKind::Telemetry;
        }
        self.enqueue_records(batch.records)
    }

    fn flush(&mut self) -> Result<FlushReport, TelemetrySinkError> {
        Ok(FlushReport {
            attempted: 0,
            published: 0,
            remaining: self
                .chunks
                .iter()
                .fold(0_usize, |acc, chunk| acc.saturating_add(chunk.record_count)),
            dropped: self.dropped_records,
        })
    }

    fn replay_from(
        &self,
        sequence_exclusive: u64,
        limit: usize,
    ) -> Result<Vec<SinkRecord>, TelemetrySinkError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let mut records = Vec::new();
        for chunk in &self.chunks {
            if chunk.max_sequence <= sequence_exclusive {
                continue;
            }
            let mut chunk_records = read_archive_chunk(&chunk.path)?;
            chunk_records.retain(|record| record.sequence > sequence_exclusive);
            records.extend(chunk_records);
            if records.len() >= limit {
                break;
            }
        }
        records.sort_by_key(|record| record.sequence);
        records.truncate(limit);
        Ok(records)
    }
}

impl AuditSink for DiskArchiveSink {
    fn emit_event(&mut self, event: AuditEvent) -> Result<(), TelemetrySinkError> {
        self.enqueue_records(vec![audit_event_to_record(event)?])
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedRecord {
    record: SinkRecord,
    checksum: String,
}

fn sanitize_record(record: &mut SinkRecord) {
    record.topic = record.topic.trim().to_string();
    record.actor = sanitize_optional_text(record.actor.clone());
    record.reason = sanitize_optional_text(record.reason.clone());
    record.committed_configuration_id =
        sanitize_optional_text(record.committed_configuration_id.clone());
    record.working_configuration_id =
        sanitize_optional_text(record.working_configuration_id.clone());
}

fn sanitize_optional_text(value: Option<String>) -> Option<String> {
    value.and_then(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn audit_event_to_record(event: AuditEvent) -> Result<SinkRecord, TelemetrySinkError> {
    if event.actor.trim().is_empty() {
        return Err(TelemetrySinkError::new(
            E_RUNTIME_TELEMETRY_RECORD_CORRUPT,
            "audit actor must be non-empty",
            false,
        ));
    }
    Ok(SinkRecord {
        sequence: event.sequence,
        timestamp_unix_ms: event.timestamp_unix_ms,
        topic: event.topic.trim().to_string(),
        kind: SinkRecordKind::Audit,
        payload: event.payload,
        actor: Some(event.actor.trim().to_string()),
        reason: sanitize_optional_text(event.reason),
        committed_configuration_id: sanitize_optional_text(event.committed_configuration_id),
        working_configuration_id: sanitize_optional_text(event.working_configuration_id),
    })
}

fn record_checksum(record: &SinkRecord) -> Result<String, TelemetrySinkError> {
    let payload = serde_json::to_vec(record).map_err(|error| {
        TelemetrySinkError::new(
            E_RUNTIME_TELEMETRY_RECORD_CORRUPT,
            format!("Failed to serialize sink record for checksum: {error}"),
            false,
        )
    })?;
    Ok(sha256_hex(&payload))
}

fn serialize_persisted_record(record: &SinkRecord) -> Result<Vec<u8>, TelemetrySinkError> {
    let persisted = PersistedRecord {
        record: record.clone(),
        checksum: record_checksum(record)?,
    };
    serde_json::to_vec(&persisted).map_err(|error| {
        TelemetrySinkError::new(
            E_RUNTIME_TELEMETRY_RECORD_CORRUPT,
            format!("Failed to serialize persisted sink record: {error}"),
            false,
        )
    })
}

fn parse_persisted_record(line: &[u8]) -> Result<PersistedRecord, TelemetrySinkError> {
    serde_json::from_slice(line).map_err(|error| {
        TelemetrySinkError::new(
            E_RUNTIME_TELEMETRY_RECORD_CORRUPT,
            format!("Failed to parse persisted sink record: {error}"),
            false,
        )
    })
}

fn queue_serialized_bytes(queue: &VecDeque<SinkRecord>) -> u64 {
    queue
        .iter()
        .filter_map(|record| serialize_persisted_record(record).ok())
        .fold(0_u64, |acc, line| acc.saturating_add(line.len() as u64 + 1))
}

fn persist_spool_queue(
    spool_path: &Path,
    queue: &VecDeque<SinkRecord>,
) -> Result<(), TelemetrySinkError> {
    if queue.is_empty() {
        if spool_path.exists() {
            fs::remove_file(spool_path).map_err(|error| {
                TelemetrySinkError::new(
                    E_RUNTIME_TELEMETRY_IO,
                    format!(
                        "Failed to remove deferred spool '{}': {}",
                        spool_path.display(),
                        error
                    ),
                    true,
                )
            })?;
        }
        return Ok(());
    }

    let temp_path = spool_path.with_extension("tmp");
    let mut file = File::create(&temp_path).map_err(|error| {
        TelemetrySinkError::new(
            E_RUNTIME_TELEMETRY_IO,
            format!(
                "Failed to create deferred spool temp file '{}': {}",
                temp_path.display(),
                error
            ),
            true,
        )
    })?;
    for record in queue {
        let mut line = serialize_persisted_record(record)?;
        line.push(b'\n');
        file.write_all(&line).map_err(|error| {
            TelemetrySinkError::new(
                E_RUNTIME_TELEMETRY_IO,
                format!(
                    "Failed to write deferred spool temp file '{}': {}",
                    temp_path.display(),
                    error
                ),
                true,
            )
        })?;
    }
    file.flush().map_err(|error| {
        TelemetrySinkError::new(
            E_RUNTIME_TELEMETRY_IO,
            format!(
                "Failed to flush deferred spool temp file '{}': {}",
                temp_path.display(),
                error
            ),
            true,
        )
    })?;
    fs::rename(&temp_path, spool_path).map_err(|error| {
        TelemetrySinkError::new(
            E_RUNTIME_TELEMETRY_IO,
            format!(
                "Failed to atomically replace deferred spool '{}': {}",
                spool_path.display(),
                error
            ),
            true,
        )
    })
}

fn load_spool_queue(spool_path: &Path) -> Result<VecDeque<SinkRecord>, TelemetrySinkError> {
    if !spool_path.exists() {
        return Ok(VecDeque::new());
    }

    let bytes = fs::read(spool_path).map_err(|error| {
        TelemetrySinkError::new(
            E_RUNTIME_TELEMETRY_IO,
            format!(
                "Failed to read deferred spool '{}': {}",
                spool_path.display(),
                error
            ),
            true,
        )
    })?;

    let mut queue = VecDeque::new();
    for line in bytes.split(|byte| *byte == b'\n') {
        if line.is_empty() {
            continue;
        }
        let parsed = match parse_persisted_record(line) {
            Ok(value) => value,
            Err(_) => break,
        };
        let expected_checksum = match record_checksum(&parsed.record) {
            Ok(checksum) => checksum,
            Err(_) => break,
        };
        if parsed.checksum != expected_checksum {
            break;
        }
        queue.push_back(parsed.record);
    }
    Ok(queue)
}

fn write_archive_chunk(path: &Path, records: &[SinkRecord]) -> Result<(), TelemetrySinkError> {
    let file = File::create(path).map_err(|error| {
        TelemetrySinkError::new(
            E_RUNTIME_TELEMETRY_IO,
            format!("Failed to create archive chunk '{}': {}", path.display(), error),
            true,
        )
    })?;
    let mut encoder = GzEncoder::new(file, Compression::default());
    for record in records {
        let mut line = serialize_persisted_record(record)?;
        line.push(b'\n');
        encoder.write_all(&line).map_err(|error| {
            TelemetrySinkError::new(
                E_RUNTIME_TELEMETRY_IO,
                format!("Failed to write archive chunk '{}': {}", path.display(), error),
                true,
            )
        })?;
    }
    encoder.finish().map_err(|error| {
        TelemetrySinkError::new(
            E_RUNTIME_TELEMETRY_IO,
            format!("Failed to finalize archive chunk '{}': {}", path.display(), error),
            true,
        )
    })?;
    Ok(())
}

fn read_archive_chunk(path: &Path) -> Result<Vec<SinkRecord>, TelemetrySinkError> {
    let file = File::open(path).map_err(|error| {
        TelemetrySinkError::new(
            E_RUNTIME_TELEMETRY_IO,
            format!("Failed to open archive chunk '{}': {}", path.display(), error),
            true,
        )
    })?;
    let decoder = GzDecoder::new(file);
    let mut reader = BufReader::new(decoder);
    let mut records = Vec::new();
    let mut line = Vec::new();
    loop {
        line.clear();
        let read = reader.read_until(b'\n', &mut line).map_err(|error| {
            TelemetrySinkError::new(
                E_RUNTIME_TELEMETRY_IO,
                format!("Failed to read archive chunk '{}': {}", path.display(), error),
                true,
            )
        })?;
        if read == 0 {
            break;
        }
        if line.last().copied() == Some(b'\n') {
            line.pop();
        }
        if line.is_empty() {
            continue;
        }
        let parsed = match parse_persisted_record(&line) {
            Ok(value) => value,
            Err(_) => break,
        };
        let expected_checksum = match record_checksum(&parsed.record) {
            Ok(checksum) => checksum,
            Err(_) => break,
        };
        if parsed.checksum != expected_checksum {
            break;
        }
        records.push(parsed.record);
    }
    Ok(records)
}

fn parse_archive_chunk_filename(path: &Path) -> Option<(u64, u64, u64)> {
    let name = path.file_name()?.to_str()?;
    if !name.starts_with("chunk-") || !name.ends_with(".jsonl.gz") {
        return None;
    }
    let mut parts = name.split('-');
    let _prefix = parts.next()?;
    let min_sequence = parts.next()?.parse::<u64>().ok()?;
    let max_sequence = parts.next()?.parse::<u64>().ok()?;
    let counter_with_suffix = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    let file_counter = counter_with_suffix
        .strip_suffix(".jsonl.gz")?
        .parse::<u64>()
        .ok()?;
    Some((min_sequence, max_sequence, file_counter))
}

fn load_archive_chunks(archive_dir: &Path) -> Result<VecDeque<ArchiveChunk>, TelemetrySinkError> {
    let mut chunks = Vec::new();
    let read_dir = fs::read_dir(archive_dir).map_err(|error| {
        TelemetrySinkError::new(
            E_RUNTIME_TELEMETRY_IO,
            format!(
                "Failed to enumerate archive directory '{}': {}",
                archive_dir.display(),
                error
            ),
            true,
        )
    })?;
    for entry in read_dir {
        let entry = entry.map_err(|error| {
            TelemetrySinkError::new(
                E_RUNTIME_TELEMETRY_IO,
                format!(
                    "Failed to read archive directory entry in '{}': {}",
                    archive_dir.display(),
                    error
                ),
                true,
            )
        })?;
        let path = entry.path();
        if path.extension() != Some(OsStr::new("gz")) {
            continue;
        }
        let Some((min_sequence, max_sequence, file_counter)) = parse_archive_chunk_filename(&path)
        else {
            continue;
        };
        let size_bytes = fs::metadata(&path)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        let record_count = read_archive_chunk(&path).map(|records| records.len())?;
        chunks.push(ArchiveChunk {
            path,
            min_sequence,
            max_sequence,
            file_counter,
            size_bytes,
            record_count,
        });
    }
    chunks.sort_by(|left, right| {
        (
            left.min_sequence,
            left.max_sequence,
            left.file_counter,
            &left.path,
        )
            .cmp(&(
                right.min_sequence,
                right.max_sequence,
                right.file_counter,
                &right.path,
            ))
    });
    Ok(VecDeque::from(chunks))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenario_test_support::{unique_temp_dir, TempDirGuard};

    fn temp_dir(label: &str) -> TempDirGuard {
        unique_temp_dir("configflux-telemetry-sink", label).expect("create temp dir")
    }

    #[derive(Default)]
    struct ScriptedPublisher {
        responses: VecDeque<Result<(), TelemetrySinkError>>,
        published_batches: Vec<Vec<SinkRecord>>,
    }

    impl ScriptedPublisher {
        fn with_responses(responses: Vec<Result<(), TelemetrySinkError>>) -> Self {
            Self {
                responses: responses.into_iter().collect(),
                published_batches: Vec::new(),
            }
        }
    }

    impl TelemetryPublisher for ScriptedPublisher {
        fn publish(&mut self, records: &[SinkRecord]) -> Result<(), TelemetrySinkError> {
            self.published_batches.push(records.to_vec());
            self.responses.pop_front().unwrap_or(Ok(()))
        }
    }

    fn telemetry_record(sequence: u64) -> SinkRecord {
        SinkRecord {
            sequence,
            timestamp_unix_ms: sequence.saturating_mul(1000),
            topic: "cfg/device/telemetry".to_string(),
            kind: SinkRecordKind::Telemetry,
            payload: serde_json::json!({
                "seq": sequence,
                "value": sequence as i64,
            }),
            actor: None,
            reason: None,
            committed_configuration_id: None,
            working_configuration_id: None,
        }
    }

    #[test]
    fn buffered_push_sink_is_bounded_and_flushes_deterministically() {
        let publisher = ScriptedPublisher::with_responses(vec![Ok(()), Ok(())]);
        let mut sink = BufferedPushSink::new(publisher, 3, 2);
        sink.emit_batch(TelemetryBatch {
            records: vec![
                telemetry_record(1),
                telemetry_record(2),
                telemetry_record(3),
                telemetry_record(4),
            ],
        })
        .expect("emit batch");

        let replay = sink.replay_from(0, 10).expect("replay");
        assert_eq!(replay.len(), 3);
        assert_eq!(replay[0].sequence, 2);
        assert_eq!(replay[1].sequence, 3);
        assert_eq!(replay[2].sequence, 4);
        assert_eq!(sink.dropped_records(), 1);

        let report = sink.flush().expect("flush");
        assert_eq!(report.attempted, 3);
        assert_eq!(report.published, 3);
        assert_eq!(report.remaining, 0);
        assert_eq!(report.dropped, 1);
        assert_eq!(sink.publisher.published_batches.len(), 2);
        assert_eq!(
            sink.publisher.published_batches[0]
                .iter()
                .map(|record| record.sequence)
                .collect::<Vec<_>>(),
            vec![2, 3]
        );
        assert_eq!(
            sink.publisher.published_batches[1]
                .iter()
                .map(|record| record.sequence)
                .collect::<Vec<_>>(),
            vec![4]
        );
    }

    #[test]
    fn deferred_upload_sink_persists_and_recovers_queue_state() {
        let temp = temp_dir("deferred");
        let spool_path = temp.path.join("deferred.spool.jsonl");

        let publisher = ScriptedPublisher::with_responses(vec![Err(TelemetrySinkError::new(
            E_RUNTIME_TELEMETRY_PUBLISH_FAILED,
            "link down",
            true,
        ))]);
        let mut sink = DeferredUploadSink::open(&spool_path, publisher, 16, 64 * 1024, 4)
            .expect("open sink");
        sink.emit_batch(TelemetryBatch {
            records: vec![telemetry_record(10), telemetry_record(11)],
        })
        .expect("emit batch");
        let flush_error = sink.flush().expect_err("flush should fail");
        assert_eq!(flush_error.code, E_RUNTIME_TELEMETRY_PUBLISH_FAILED);
        assert_eq!(sink.replay_from(0, 10).expect("replay").len(), 2);

        let replay_publisher = ScriptedPublisher::default();
        let mut replay_sink =
            DeferredUploadSink::open(&spool_path, replay_publisher, 16, 64 * 1024, 4)
                .expect("reopen sink");
        let replayed = replay_sink.replay_from(0, 10).expect("replay after restart");
        assert_eq!(
            replayed.iter().map(|record| record.sequence).collect::<Vec<_>>(),
            vec![10, 11]
        );
        let report = replay_sink.flush().expect("flush after restart");
        assert_eq!(report.published, 2);
        assert_eq!(report.remaining, 0);
        assert!(!spool_path.exists());
    }

    #[test]
    fn disk_archive_sink_compresses_replays_and_prunes_oldest_chunks() {
        let temp = temp_dir("archive");
        let mut sink = DiskArchiveSink::open(&temp.path, 2, 10 * 1024 * 1024).expect("open sink");

        sink.emit_batch(TelemetryBatch {
            records: vec![telemetry_record(1), telemetry_record(2)],
        })
        .expect("emit chunk one");
        sink.emit_batch(TelemetryBatch {
            records: vec![telemetry_record(3)],
        })
        .expect("emit chunk two");
        sink.emit_batch(TelemetryBatch {
            records: vec![telemetry_record(4)],
        })
        .expect("emit chunk three");

        let entries: Vec<PathBuf> = fs::read_dir(&temp.path)
            .expect("read archive dir")
            .map(|entry| entry.expect("entry").path())
            .collect();
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().all(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.ends_with(".jsonl.gz"))
                .unwrap_or(false)
        }));

        let replay = sink.replay_from(0, 10).expect("replay archive");
        assert_eq!(
            replay.iter().map(|record| record.sequence).collect::<Vec<_>>(),
            vec![3, 4]
        );
        assert!(sink.dropped_records() >= 1);
    }
}
