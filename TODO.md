Investigate and fix the following issues across web, Android, server, Docker, import pipeline, and AI processing for a photo library app. The library size is ~100 GB and the app currently shows ~75 GB after import. Prioritize data integrity, sync performance, upload reliability, AI stability, and video playback. For each issue below, produce: root cause hypothesis, exact repro steps, required logs and artifacts, code-level fixes or configuration changes, tests to validate the fix, performance benchmarks, and rollout plan.

Issues:
1. Import size mismatch: reported size lower than actual; possible missing files or lossy conversion.
2. Photo viewer double-tap zoom toggle not returning to original zoom.
3. Android sync too slow; appears to transfer large payloads; initial visibility takes ~20 minutes.
4. Uploads from Android failing intermittently; investigate Docker and server config.
5. AI facial detection fails on large libraries and causes server crashes.
6. Geo location: infer home location from frequent coordinates and add manual home address input in settings; exclude home photos from trip counts.
7. Android scrolling lag on large libraries; likely not unloading off-screen images.
8. App reopen sync delay: gallery not viewable for minutes after reopening.
9. Swipe sensitivity while zoomed: panning triggers info panel or closes photo.
10. Video playback buffering long, black screen, format errors for valid MP4s; download button unresponsive while loading.
11. Google Takeout import should recreate albums and apply photo edits.
12. Albums on Android do not auto-refresh; web albums and gallery repopulate each load suggesting missing caching.
13. AI processing does not handle videos or GIFs and rescans cause server instability.
14. Confirm all photoviewers options are working correctly and aligned, issues with reecent aalbums photos missing eedit,favorites, and tags options on android, did not check on web. this is probally an alignment issue.

Acceptance criteria:
- Import size within 2% of disk usage; file counts match source.
- Thumbnails visible within 10 seconds; incremental sync under 2 minutes for recent changes.
- Upload success rate ≥99%.
- No server crashes during AI processing; processing throughput documented.
- Video first-frame under 2 seconds on LAN; no false format errors.
- Albums and edits restored for Google Takeout imports.
- Double-tap toggles zoom; swipe thresholds adjusted to avoid accidental actions.
- About page version auto-updates; Manage Users server IP links to webserver.

Collect these artifacts: server and Docker logs, transcode logs, AI job queue snapshots, Android logcat, network traces, sample media, and Google Takeout archive.

Prioritize fixes in this order: import integrity, sync performance, uploads, AI stability, video playback, caching and memory, UX gesture fixes, Google Takeout fidelity, versioning and links.

Produce a step-by-step remediation plan, code patches, tests, monitoring changes, and a rollback strategy.

