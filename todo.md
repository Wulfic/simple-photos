FORMAT:
- Each item: ID | Title | Priority | Area | Owner | Estimate
- Followed by: Acceptance Criteria, Implementation Steps, Dependencies, Risks
- Use this file to create tickets or feed to your AI task runner.

1 | Unify banner counts and ETA across clients
Priority: High
Area: Backend / Web / Android
Owner: Backend + Web + Android
Estimate: 5d
Acceptance Criteria:
- Single authoritative server API provides aggregated encryption totals and ETA.
- Android local-upload counts are added to server total before display.
- Web and Android show identical totals and ETA within 1% variance.
Implementation Steps:
- Design API endpoint /status/encryption returning totals, per-source contributions, eta_seconds.
- Add client contribution endpoint or include counts in client heartbeat.
- Server aggregates contributions and computes ETA using throughput estimator.
- Implement real-time updates via SSE/WebSocket; clients subscribe.
- Update UI to display server totals; per-source breakdown only in debug mode.
Dependencies:
- Backend API, real-time channel, Android and Web client updates.
Risks:
- Spoofed client contributions; require auth and rate limiting.

2 | Merge Android local upload counts into server total
Priority: High
Area: Android / Backend
Owner: Android + Backend
Estimate: 2d
Acceptance Criteria:
- Android sends pending local upload count to server.
- Server aggregates and returns unified total.
Implementation Steps:
- Add client-contrib field to heartbeat or separate endpoint.
- Server validates and aggregates.
- Clients render server-provided total.
Dependencies:
- Heartbeat mechanism, auth.
Risks:
- Network churn; ensure retries and idempotency.

3 | Fix banner overlap and responsive heights
Priority: High
Area: Web UI / Responsive
Owner: Web Frontend
Estimate: 3d
Acceptance Criteria:
- Banners stack vertically without overlap on all viewports.
- Encryption banner remains visible and readable.
Implementation Steps:
- Audit CSS stacking context and z-index.
- Implement single banner container with vertical stacking and consistent padding.
- Add max-height and internal scroll for many banners.
- Test on desktop and mobile breakpoints.
Dependencies:
- Web UI components.
Risks:
- Cross-browser CSS quirks.

4 | Conversation banner show remaining ETA on web
Priority: Medium
Area: Web UI
Owner: Web Frontend
Estimate: 2d
Acceptance Criteria:
- Conversation banner displays ETA from server status API and updates live.
Implementation Steps:
- Wire conversation banner to /status/encryption.
- Format ETA human-readably.
- Add tests for rendering and responsiveness.
Dependencies:
- Server status API.
Risks:
- None significant.

5 | Align Android banners with web visuals and behavior
Priority: High
Area: Android UI
Owner: Android Team
Estimate: 3d
Acceptance Criteria:
- Visual parity and identical data source for banners.
- Same ordering and copy.
Implementation Steps:
- Use server aggregation endpoint.
- Standardize banner component props and copy.
- QA across devices.
Dependencies:
- Backend API, design tokens.
Risks:
- Device-specific layout differences.

6 | Recolor edit panel buttons to theme
Priority: Low
Area: UI / Theming
Owner: Web + Android
Estimate: 1d
Acceptance Criteria:
- Edit panel uses theme primary/secondary colors.
- Red reserved for destructive irreversible actions.
Implementation Steps:
- Replace special green/red tokens with theme tokens.
- Run accessibility contrast checks.
Dependencies:
- Design tokens.
Risks:
- Visual regressions; run visual tests.

7 | Fix panoramic detection false positives
Priority: High
Area: Media Processing / AI
Owner: Vision Team
Estimate: 4d
Acceptance Criteria:
- False positives for landscape photos reduced by ≥90%.
- AI-enabled path uses classifier; AI-disabled uses loose dimension heuristic.
Implementation Steps:
- Integrate lightweight panorama classifier (on-device or server).
- Pipeline: if AI enabled → classifier; if confidence low → fallback to heuristic.
- Heuristic: use EXIF orientation and aspect ratio thresholds (looser when AI disabled).
- Add manual override and telemetry for misflags.
Dependencies:
- Model hosting or on-device inference, EXIF extraction.
Risks:
- Privacy if server-side inference used; prefer local inference.

8 | Use 120th frame for facial detection in videos
Priority: Medium
Area: Video Processing
Owner: Media Pipeline
Estimate: 3d
Acceptance Criteria:
- Face detection runs on frame at ~5s (fps * 5).
- If no face, fallback to sliding window ±2s.
Implementation Steps:
- Extract frame at frame_index = clamp(fps * 5).
- Run face detector; if none, search ±48 frames.
- Store chosen frame id and face metadata.
Dependencies:
- Video processing pipeline, face detector.
Risks:
- Variable fps; clamp to available frames.

9 | Confirm and fix Android background backup behavior
Priority: High
Area: Android Behavior / Background Tasks
Owner: Android Team
Estimate: 2d
Acceptance Criteria:
- Documented behavior for background backups across supported Android versions.
- If unreliable, implement WorkManager foreground service with proper notification.
Implementation Steps:
- Audit current implementation (WorkManager, foreground service).
- Test under Doze and battery optimizations.
- Implement foreground service or setForegroundAsync if needed.
- Update docs and privacy notes.
Dependencies:
- Android APIs, QA devices.
Risks:
- Battery impact; require user-visible notification.

10 | Implement secure heartbeat for local self-healing over port 3301
Priority: High
Area: Networking / Security
Owner: Backend + Security
Estimate: 4d
Acceptance Criteria:
- Heartbeat every 15 minutes with encrypted, authenticated payload.
- Reconnection attempts on missed heartbeats.
- Security review completed.
Implementation Steps:
- Define minimal encrypted payload with timestamp and nonce.
- Use mutual TLS or HMAC for authentication.
- Implement sender/receiver and reconnection logic with exponential backoff.
- Security review for interception and replay protection.
Dependencies:
- TLS/HMAC key management, networking stack.
Risks:
- Exposed port risk; require strong auth and encryption.

11 | Keep Albums and Gallery in sync with server
Priority: High
Area: Sync / Data
Owner: Backend + Mobile
Estimate: 5d
Acceptance Criteria:
- Server changes propagate to clients within target windows (active <1m, background <5m).
- Conflict resolution rules documented.
Implementation Steps:
- Implement server push (SSE/WebSocket) for album changes.
- Clients subscribe and update local DB.
- Background sync fallback for offline clients.
- Define conflict resolution (timestamp-based).
Dependencies:
- Real-time channel, client DB sync logic.
Risks:
- Sync storms; implement rate limiting.

12 | Queue conversions to avoid blocking encryption
Priority: High
Area: Job Scheduling / Import Pipeline
Owner: Backend
Estimate: 3d
Acceptance Criteria:
- Conversions do not block encryption; conversions run after encryption completes for the batch.
- Facial and geo processing run after conversions.
Implementation Steps:
- Tag jobs as encryption, conversion, post-processing.
- Ensure encryption queue priority; conversions queued to conversion queue.
- Trigger post-processing after conversion completion.
Dependencies:
- Job scheduler, queue system.
Risks:
- Ordering bugs; add tests.

13 | Add scrollbars to gallery pages
Priority: Medium
Area: Web UI
Owner: Web Frontend
Estimate: 2d
Acceptance Criteria:
- Visible, usable scrollbars for large libraries; touch and keyboard support.
Implementation Steps:
- Add CSS overflow and accessible custom scrollbar styles.
- Test with very large datasets.
Dependencies:
- UI components.
Risks:
- Cross-platform scrollbar behavior.

14 | Fix Android thumbnail size toggle UI and functionality
Priority: Medium
Area: Android UI
Owner: Android Team
Estimate: 2d
Acceptance Criteria:
- Toggle label horizontal and localized.
- Toggle changes thumbnail size immediately and persists preference.
Implementation Steps:
- Fix layout XML orientation.
- Hook toggle to preference and refresh grid.
- Add UI tests.
Dependencies:
- Localization strings.
Risks:
- None significant.

15 | Correct import dates parsing from Google Takeout
Priority: High
Area: Importer
Owner: Backend Import Team
Estimate: 3d
Acceptance Criteria:
- Imported items show correct creation and modification dates from Takeout metadata.
- Timezone-aware normalization.
Implementation Steps:
- Inspect Takeout JSON fields (photoTakenTime, creationTime).
- Prefer explicit timestamps over filesystem dates.
- Normalize to UTC and store original timezone if present.
- Add tests with sample exports.
Dependencies:
- Sample Takeout exports.
Risks:
- Inconsistent metadata formats; add robust parsing.

16 | Stabilize server during AI processing on large libraries
Priority: Critical
Area: Backend / AI Infrastructure
Owner: Backend + AI Infra
Estimate: 7d
Acceptance Criteria:
- No crashes under representative load.
- AI jobs rate-limited and queued; resource usage bounded.
Implementation Steps:
- Profile AI jobs for memory/CPU hotspots.
- Introduce worker pools, batching, throttling, and circuit breaker.
- Implement autoscaling or offload to managed inference.
- Add monitoring and alerts.
Dependencies:
- AI infra, autoscaling, monitoring.
Risks:
- Cost of autoscaling; plan capacity.

17 | Fix Android lock when landscape with biometrics enabled
Priority: High
Area: Android Stability
Owner: Android Team
Estimate: 2d
Acceptance Criteria:
- App does not lock unexpectedly in landscape while biometric session active.
Implementation Steps:
- Reproduce across devices.
- Inspect lifecycle and biometric prompt handling.
- Fix orientation-related lifecycle or prompt dismissal logic.
- Add regression tests.
Dependencies:
- Biometric library versions.
Risks:
- Device-specific behavior.

18 | Detect and recover stuck conversion jobs
Priority: High
Area: Job Management
Owner: Backend
Estimate: 2d
Acceptance Criteria:
- Jobs stuck >8 hours are marked stalled, logged, and retried or escalated.
- System resumes subsequent jobs.
Implementation Steps:
- Add job watchdog to check runtime and state.
- On threshold exceed, capture logs, mark stalled, attempt safe rollback, requeue or escalate.
- Provide admin UI for manual intervention.
Dependencies:
- Job store, admin UI.
Risks:
- False positives; allow manual override.

19 | Recreate albums and dedupe on Google Takeout import
Priority: High
Area: Importer / Deduplication
Owner: Backend Import Team
Estimate: 4d
Acceptance Criteria:
- Albums from Takeout recreated with correct membership.
- When edited and original both present, prefer edited version and avoid duplicates.
Implementation Steps:
- Parse Takeout album JSON and map to internal model.
- Detect duplicates by content hash and metadata; prefer edited file if edits metadata present.
- Keep original as source link or skip storing duplicate.
- Run dedupe pass after import.
Dependencies:
- Takeout metadata, hashing pipeline.
Risks:
- Hash collisions; use robust dedupe heuristics.

PRIORITIZATION SUGGESTION:
- Start with Critical and High items: #16 Stabilize AI processing, #1 Unify banners, #11 Sync Albums/Gallery, #10 Heartbeat, #7 Panorama detection, #12 Queue conversions, #18 Unstick jobs.
- Parallelize across teams: Backend, Android, Web, AI Infra.
- Add telemetry for banner mismatches, panorama misflags, stuck jobs, import dedupe rates.
- Require security review for heartbeat and any new network endpoints.
