import { useState, useMemo, useEffect, useCallback, useRef } from "react";
import { useNavigate } from "react-router-dom";
import { QRCodeSVG } from "qrcode.react";
import { api } from "../api/client";
import { useAuthStore } from "../store/auth";
import { useBackupStore } from "../store/backup";
import { useProcessingStore } from "../store/processing";
import AppHeader from "../components/AppHeader";
import AppIcon from "../components/AppIcon";
import { checkPasswordStrength } from "../utils/validation";
import { Checkmark } from "../components/PasswordFields";
import { encrypt, sha256Hex, hasCryptoKey } from "../crypto/crypto";
import { blobTypeFromMime, mediaTypeFromMime } from "../db";

function arrayBufferToBase64(buffer: ArrayBuffer | Uint8Array): string {
  const bytes = buffer instanceof Uint8Array ? buffer : new Uint8Array(buffer);
  // Process in 8 KiB chunks to avoid O(n²) string concatenation and
  // call-stack limits with String.fromCharCode.apply on large buffers.
  const CHUNK = 8192;
  const parts: string[] = [];
  for (let i = 0; i < bytes.byteLength; i += CHUNK) {
    const slice = bytes.subarray(i, Math.min(i + CHUNK, bytes.byteLength));
    parts.push(String.fromCharCode(...slice));
  }
  return btoa(parts.join(""));
}

/**
 * Generate a JPEG thumbnail (cover-cropped to `size`x`size`) from raw image/video bytes.
 * Returns the thumbnail as an ArrayBuffer, or null if generation fails.
 */
async function generateMigrationThumbnail(
  fileData: Uint8Array,
  mimeType: string,
  size: number
): Promise<ArrayBuffer | null> {
  const blob = new Blob([fileData as BlobPart], { type: mimeType });
  const url = URL.createObjectURL(blob);

  try {
    if (mimeType.startsWith("video/")) {
      return await new Promise<ArrayBuffer | null>((resolve) => {
        const video = document.createElement("video");
        video.muted = true;
        video.playsInline = true;
        video.onloadedmetadata = () => {
          video.currentTime = Math.min(Math.max(video.duration * 0.1, 1), video.duration);
        };
        video.onseeked = () => {
          URL.revokeObjectURL(url);
          const canvas = document.createElement("canvas");
          canvas.width = size;
          canvas.height = size;
          const ctx = canvas.getContext("2d")!;
          const scale = Math.max(size / video.videoWidth, size / video.videoHeight);
          const w = video.videoWidth * scale;
          const h = video.videoHeight * scale;
          ctx.drawImage(video, (size - w) / 2, (size - h) / 2, w, h);
          canvas.toBlob(
            (b) => (b ? b.arrayBuffer().then((ab) => resolve(ab)) : resolve(null)),
            "image/jpeg",
            0.8
          );
        };
        video.onerror = () => { URL.revokeObjectURL(url); resolve(null); };
        // Timeout: if video doesn't load in 10 s, skip thumbnail
        setTimeout(() => { URL.revokeObjectURL(url); resolve(null); }, 10_000);
        video.src = url;
      });
    }

    // Image path
    return await new Promise<ArrayBuffer | null>((resolve) => {
      const img = new Image();
      img.onload = () => {
        URL.revokeObjectURL(url);
        const canvas = document.createElement("canvas");
        canvas.width = size;
        canvas.height = size;
        const ctx = canvas.getContext("2d")!;
        const scale = Math.max(size / img.naturalWidth, size / img.naturalHeight);
        const w = img.naturalWidth * scale;
        const h = img.naturalHeight * scale;
        ctx.drawImage(img, (size - w) / 2, (size - h) / 2, w, h);
        canvas.toBlob(
          (b) => (b ? b.arrayBuffer().then((ab) => resolve(ab)) : resolve(null)),
          "image/jpeg",
          0.8
        );
      };
      img.onerror = () => { URL.revokeObjectURL(url); resolve(null); };
      img.src = url;
    });
  } catch {
    URL.revokeObjectURL(url);
    return null;
  }
}

// Minimal role check: decode JWT payload to see if user is admin
function useIsAdmin(): boolean {
  const { accessToken } = useAuthStore();
  if (!accessToken) return false;
  try {
    const payload = JSON.parse(atob(accessToken.split(".")[1]));
    return payload.role === "admin";
  } catch {
    return false;
  }
}

export default function Settings() {
  const { username } = useAuthStore();
  const isAdmin = useIsAdmin();
  const { startTask, endTask } = useProcessingStore();
  const navigate = useNavigate();

  // ── 2FA state ────────────────────────────────────────────────────────────
  const [totpUri, setTotpUri] = useState<string | null>(null);
  const [backupCodes, setBackupCodes] = useState<string[]>([]);
  const [totpCode, setTotpCode] = useState("");
  const [disableCode, setDisableCode] = useState("");
  const [showDisable2fa, setShowDisable2fa] = useState(false);

  // ── Password change state ────────────────────────────────────────────────
  const [showChangePassword, setShowChangePassword] = useState(false);
  const [currentPassword, setCurrentPassword] = useState("");
  const [newPassword, setNewPassword] = useState("");
  const [confirmNewPassword, setConfirmNewPassword] = useState("");

  // ── General state ────────────────────────────────────────────────────────
  const [error, setError] = useState("");
  const [success, setSuccess] = useState("");
  const [loading, setLoading] = useState(false);

  // ── Encryption state ─────────────────────────────────────────────────────
  const [encryptionMode, setEncryptionMode] = useState<"plain" | "encrypted">("plain");
  const [migrationStatus, setMigrationStatus] = useState("idle");
  const [migrationTotal, setMigrationTotal] = useState(0);
  const [migrationCompleted, setMigrationCompleted] = useState(0);
  const [migrationError, setMigrationError] = useState<string | null>(null);
  const [encryptionLoading, setEncryptionLoading] = useState(true);
  const [togglingEncryption, setTogglingEncryption] = useState(false);
  const [showEncryptionWarning, setShowEncryptionWarning] = useState(false);

  // ── Backup recovery state ────────────────────────────────────────────────
  const [showRecoverWarning, setShowRecoverWarning] = useState(false);
  const { backupServers, loaded: backupLoaded, recovering, setRecovering, setBackupServers, setLoaded: setBackupLoaded, viewMode, setViewMode, activeBackupServerId, setActiveBackupServerId } = useBackupStore();

  // ── SSL / TLS state (admin only) ──────────────────────────────────────────
  const [sslEnabled, setSslEnabled] = useState(false);
  const [sslCertPath, setSslCertPath] = useState("");
  const [sslKeyPath, setSslKeyPath] = useState("");
  const [sslLoaded, setSslLoaded] = useState(false);
  const [sslSaving, setSslSaving] = useState(false);
  const [sslSaved, setSslSaved] = useState(false);
  const [sslMode, setSslMode] = useState<"manual" | "letsencrypt">("manual");
  const [leDomain, setLeDomain] = useState("");
  const [leEmail, setLeEmail] = useState("");
  const [leStaging, setLeStaging] = useState(false);
  const [leGenerating, setLeGenerating] = useState(false);
  const [leGenerated, setLeGenerated] = useState(false);
  const [leError, setLeError] = useState<string | null>(null);
  const [leStatusLog, setLeStatusLog] = useState<string[]>([]);

  // ── Manual backup server state ────────────────────────────────────────────
  const [showAddBackupServer, setShowAddBackupServer] = useState(false);
  const [backupServerName, setBackupServerName] = useState("");
  const [backupServerAddress, setBackupServerAddress] = useState("");
  const [backupServerApiKey, setBackupServerApiKey] = useState("");
  const [backupServerFrequency, setBackupServerFrequency] = useState("24");
  const [addingBackupServer, setAddingBackupServer] = useState(false);

  // ── User Management state (admin only) ─────────────────────────────────
  type ManagedUser = { id: string; username: string; role: string; totp_enabled: boolean; created_at: string };
  const [managedUsers, setManagedUsers] = useState<ManagedUser[]>([]);
  const [usersLoaded, setUsersLoaded] = useState(false);
  const [showAddUser, setShowAddUser] = useState(false);
  const [newUsername, setNewUsername] = useState("");
  const [newUserPassword, setNewUserPassword] = useState("");
  const [newUserRole, setNewUserRole] = useState<"user" | "admin">("user");
  const [resetPwUserId, setResetPwUserId] = useState<string | null>(null);
  const [resetPwValue, setResetPwValue] = useState("");
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);

  // ── Scan state (admin, plain mode) ──────────────────────────────────────
  const [scanning, setScanning] = useState(false);
  const [scanResult, setScanResult] = useState<string | null>(null);

  // ── Storage stats state ─────────────────────────────────────────────────
  type StorageStats = {
    photo_bytes: number; photo_count: number;
    video_bytes: number; video_count: number;
    other_blob_bytes: number; other_blob_count: number;
    plain_bytes: number; plain_count: number;
    user_total_bytes: number;
    fs_total_bytes: number; fs_free_bytes: number;
  };
  const [storageStats, setStorageStats] = useState<StorageStats | null>(null);
  const [storageLoading, setStorageLoading] = useState(true);

  // ── Admin 2FA setup state ───────────────────────────────────────────────
  const [setup2faUserId, setSetup2faUserId] = useState<string | null>(null);
  const [setup2faUri, setSetup2faUri] = useState<string | null>(null);
  const [setup2faBackupCodes, setSetup2faBackupCodes] = useState<string[]>([]);
  const [setup2faCode, setSetup2faCode] = useState("");
  const [setup2faLoading, setSetup2faLoading] = useState(false);

  const pw = useMemo(() => checkPasswordStrength(newPassword), [newPassword]);

  // ── 2FA handlers ─────────────────────────────────────────────────────────

  async function handleSetup2fa() {
    setError("");
    setSuccess("");
    setLoading(true);
    try {
      const res = await api.auth.setup2fa();
      setTotpUri(res.otpauth_uri);
      setBackupCodes(res.backup_codes);
    } catch (err: any) {
      setError(err.message);
    } finally {
      setLoading(false);
    }
  }

  async function handleConfirm2fa(e: React.FormEvent) {
    e.preventDefault();
    setError("");
    setLoading(true);
    try {
      await api.auth.confirm2fa(totpCode);
      setSuccess("Two-factor authentication enabled successfully!");
      setTotpUri(null);
      setTotpCode("");
    } catch (err: any) {
      setError(err.message);
    } finally {
      setLoading(false);
    }
  }

  async function handleDisable2fa(e: React.FormEvent) {
    e.preventDefault();
    setError("");
    setLoading(true);
    try {
      await api.auth.disable2fa(disableCode);
      setSuccess("Two-factor authentication disabled.");
      setShowDisable2fa(false);
      setDisableCode("");
    } catch (err: any) {
      setError(err.message);
    } finally {
      setLoading(false);
    }
  }

  // ── Password change handler ──────────────────────────────────────────────

  async function handleChangePassword(e: React.FormEvent) {
    e.preventDefault();
    setError("");
    setSuccess("");

    if (!pw.core) {
      setError(
        "New password must be at least 8 characters with uppercase, lowercase, and a digit."
      );
      return;
    }
    if (newPassword !== confirmNewPassword) {
      setError("New passwords do not match.");
      return;
    }
    if (currentPassword === newPassword) {
      setError("New password must be different from current password.");
      return;
    }

    setLoading(true);
    try {
      await api.auth.changePassword(currentPassword, newPassword);
      setSuccess(
        "Password changed successfully. All other sessions have been revoked."
      );
      setShowChangePassword(false);
      setCurrentPassword("");
      setNewPassword("");
      setConfirmNewPassword("");
    } catch (err: any) {
      setError(err.message);
    } finally {
      setLoading(false);
    }
  }

  // ── Encryption handlers ──────────────────────────────────────────────────

  const loadEncryptionSettings = useCallback(async () => {
    try {
      const res = await api.encryption.getSettings();
      setEncryptionMode(res.encryption_mode as "plain" | "encrypted");
      setMigrationStatus(res.migration_status);
      setMigrationTotal(res.migration_total);
      setMigrationCompleted(res.migration_completed);
      setMigrationError(res.migration_error);
    } catch {
      // Settings might not exist yet (pre-migration)
    } finally {
      setEncryptionLoading(false);
    }
  }, []);

  // Load backup servers on mount
  const loadBackupServers = useCallback(async () => {
    try {
      const res = await api.backup.listServers();
      setBackupServers(res.servers);
    } catch {
      // Ignore if backup isn't configured
    } finally {
      setBackupLoaded(true);
    }
  }, [setBackupServers, setBackupLoaded]);

  // Fetch encryption settings and backup servers on mount
  useEffect(() => {
    loadEncryptionSettings();
    loadBackupServers();
    loadSslSettings();
    loadManagedUsers();
    loadStorageStats();
  }, [loadEncryptionSettings, loadBackupServers]);

  async function loadSslSettings() {
    try {
      const res = await api.admin.getSsl();
      setSslEnabled(res.enabled);
      setSslCertPath(res.cert_path ?? "");
      setSslKeyPath(res.key_path ?? "");
      setSslLoaded(true);
    } catch {
      // Not admin or SSL endpoints not available — silently skip
    }
  }

  async function loadStorageStats() {
    setStorageLoading(true);
    try {
      const stats = await api.storageStats.get();
      setStorageStats(stats);
    } catch {
      // Endpoint may not be available — silently skip
    } finally {
      setStorageLoading(false);
    }
  }

  // ── User Management handlers (admin only) ────────────────────────────────

  async function loadManagedUsers() {
    try {
      const users = await api.admin.listUsers();
      setManagedUsers(users);
      setUsersLoaded(true);
    } catch {
      // Not admin — silently skip
    }
  }

  async function handleAddUser(e: React.FormEvent) {
    e.preventDefault();
    setError("");
    try {
      await api.admin.createUser(newUsername, newUserPassword, newUserRole);
      setSuccess(`User "${newUsername}" created.`);
      setNewUsername("");
      setNewUserPassword("");
      setNewUserRole("user");
      setShowAddUser(false);
      await loadManagedUsers();
    } catch (err: any) {
      setError(err.message);
    }
  }

  async function handleDeleteUser(userId: string) {
    setError("");
    try {
      await api.admin.deleteUser(userId);
      setSuccess("User deleted.");
      setConfirmDeleteId(null);
      await loadManagedUsers();
    } catch (err: any) {
      setError(err.message);
    }
  }

  async function handleChangeRole(userId: string, role: "admin" | "user") {
    setError("");
    try {
      await api.admin.updateUserRole(userId, role);
      setSuccess("Role updated.");
      await loadManagedUsers();
    } catch (err: any) {
      setError(err.message);
    }
  }

  async function handleResetUserPassword(userId: string) {
    setError("");
    if (!resetPwValue || resetPwValue.length < 8) {
      setError("Password must be at least 8 characters.");
      return;
    }
    try {
      await api.admin.resetUserPassword(userId, resetPwValue);
      setSuccess("Password reset.");
      setResetPwUserId(null);
      setResetPwValue("");
    } catch (err: any) {
      setError(err.message);
    }
  }

  async function handleResetUser2fa(userId: string) {
    setError("");
    try {
      await api.admin.resetUser2fa(userId);
      setSuccess("2FA disabled for user.");
      await loadManagedUsers();
    } catch (err: any) {
      setError(err.message);
    }
  }

  async function handleAdminSetup2fa(userId: string) {
    setError("");
    setSetup2faLoading(true);
    try {
      const res = await api.admin.setupUser2fa(userId);
      setSetup2faUserId(userId);
      setSetup2faUri(res.otpauth_uri);
      setSetup2faBackupCodes(res.backup_codes);
      setSetup2faCode("");
    } catch (err: any) {
      setError(err.message || "Failed to start 2FA setup");
    } finally {
      setSetup2faLoading(false);
    }
  }

  async function handleAdminConfirm2fa() {
    if (!setup2faUserId || !setup2faCode.trim()) return;
    setError("");
    setSetup2faLoading(true);
    try {
      await api.admin.confirmUser2fa(setup2faUserId, setup2faCode.trim());
      setSuccess("2FA enabled for user.");
      setSetup2faUserId(null);
      setSetup2faUri(null);
      setSetup2faBackupCodes([]);
      setSetup2faCode("");
      await loadManagedUsers();
    } catch (err: any) {
      setError(err.message || "Invalid TOTP code");
    } finally {
      setSetup2faLoading(false);
    }
  }

  function cancelAdminSetup2fa() {
    setSetup2faUserId(null);
    setSetup2faUri(null);
    setSetup2faBackupCodes([]);
    setSetup2faCode("");
  }

  async function handleSaveSsl() {
    setSslSaving(true);
    setError("");
    try {
      await api.admin.updateSsl({
        enabled: sslEnabled,
        cert_path: sslCertPath || undefined,
        key_path: sslKeyPath || undefined,
      });
      setSslSaved(true);
      setSuccess("TLS configuration saved. Restart the server to apply changes.");
    } catch (err: any) {
      setError(err.message);
    } finally {
      setSslSaving(false);
    }
  }

  async function handleGenerateLeCert() {
    if (!leDomain.trim() || !leEmail.trim()) {
      setError("Domain and e-mail are both required.");
      return;
    }
    setLeGenerating(true);
    setLeError(null);
    setError("");
    setLeStatusLog([
      "Starting Let's Encrypt certificate generation...",
      `Domain: ${leDomain.trim()}`,
      `Contact: ${leEmail.trim()}`,
      leStaging ? "Mode: Staging (testing)" : "Mode: Production",
      "Creating ACME account...",
    ]);
    try {
      const res = await api.admin.generateLetsEncrypt({
        domain: leDomain.trim(),
        email: leEmail.trim(),
        staging: leStaging,
      });
      setLeGenerated(true);
      setSslEnabled(true);
      setSslCertPath(res.cert_path);
      setSslKeyPath(res.key_path);
      setSuccess(res.message);
      setLeStatusLog((prev) => [...prev, "Certificate generated successfully!", `Cert: ${res.cert_path}`, `Key: ${res.key_path}`]);
    } catch (err: any) {
      let msg = err.message || "Certificate generation failed";
      // Provide a more descriptive error when fetch itself fails
      if (msg === "Failed to fetch" || msg === "NetworkError when attempting to fetch resource.") {
        msg = "Could not reach the server. The request may have timed out, or the server encountered an error during certificate generation. " +
          "Ensure the server is running, the domain resolves to this server, and port 80 is accessible.";
      }
      setLeError(msg);
      setError(msg);
      setLeStatusLog((prev) => [...prev, `ERROR: ${msg}`]);
    } finally {
      setLeGenerating(false);
    }
  }

  // Poll migration progress when a migration is active
  useEffect(() => {
    if (migrationStatus !== "encrypting" && migrationStatus !== "decrypting") return;
    startTask("encryption");
    const interval = setInterval(loadEncryptionSettings, 3000);
    return () => {
      clearInterval(interval);
      endTask("encryption");
    };
  }, [migrationStatus, loadEncryptionSettings, startTask, endTask]);

  // ── Encryption migration worker ──────────────────────────────────────────
  // When the server reports an active "encrypting" migration but nothing is
  // driving it, the client-side worker downloads each plain photo, encrypts it,
  // uploads the encrypted blob, and reports progress back to the server.
  //
  // Robustness measures:
  //  - Uses api.photos.downloadFile/downloadThumb which handle 401 token refresh
  //  - Generates thumbnails client-side (avoids downloading full photo twice)
  //  - Retries each photo up to 3 times on transient failures
  //  - Tracks successes & failures separately so errors aren't lost
  const migrationRunningRef = useRef(false);

  useEffect(() => {
    if (migrationStatus !== "encrypting") return;
    if (migrationRunningRef.current) return;
    if (!hasCryptoKey()) return;

    migrationRunningRef.current = true;

    (async () => {
      try {
        console.log("[Migration] Starting encryption migration worker...");

        // Fetch ALL plain photos that need encrypting
        const allPhotos: Array<{
          id: string; filename: string; file_path: string; mime_type: string;
          media_type: string; size_bytes: number; width: number; height: number;
          duration_secs: number | null; taken_at: string | null;
          latitude: number | null; longitude: number | null;
          thumb_path: string | null; created_at: string;
        }> = [];
        let cursor: string | undefined;
        do {
          const res = await api.photos.list({ after: cursor, limit: 200 });
          allPhotos.push(...res.photos);
          cursor = res.next_cursor ?? undefined;
        } while (cursor);

        console.log(`[Migration] Found ${allPhotos.length} photos to encrypt`);

        const total = allPhotos.length;
        let completed = 0;
        let succeeded = 0;
        let failedCount = 0;
        let lastError = "";

        for (const photo of allPhotos) {
          // Retry each photo up to 3 times to handle transient network/auth issues
          let attempts = 0;
          let itemSuccess = false;

          while (attempts < 3 && !itemSuccess) {
            attempts++;
            try {
              const stepStart = Date.now();

              // Step 1: Download the raw file
              console.log(`[Migration] [${completed + 1}/${total}] "${photo.filename}" (${(photo.size_bytes / 1024).toFixed(0)} KB) — downloading (attempt ${attempts}/3)...`);
              const fileBuffer = await api.photos.downloadFile(photo.id);
              const fileData = new Uint8Array(fileBuffer);
              console.log(`[Migration]   Downloaded: ${fileData.length} bytes in ${Date.now() - stepStart}ms`);

              // Step 2: Generate thumbnail
              let thumbBlobId: string | undefined;
              try {
                const thumbStart = Date.now();
                const thumbData = await generateMigrationThumbnail(fileData, photo.mime_type, 256);
                if (thumbData) {
                  const thumbPayload = JSON.stringify({
                    v: 1,
                    photo_blob_id: "",
                    width: 256,
                    height: 256,
                    data: arrayBufferToBase64(thumbData),
                  });
                  const encThumb = await encrypt(new TextEncoder().encode(thumbPayload));
                  const thumbHash = await sha256Hex(new Uint8Array(encThumb));
                  const thumbBlobType = photo.media_type === "video" ? "video_thumbnail" : "thumbnail";
                  console.log(`[Migration]   Uploading encrypted thumbnail (${encThumb.byteLength} bytes, type=${thumbBlobType})...`);
                  const thumbUpload = await api.blobs.upload(encThumb, thumbBlobType, thumbHash);
                  thumbBlobId = thumbUpload.blob_id;
                  console.log(`[Migration]   Thumbnail uploaded: ${thumbBlobId} in ${Date.now() - thumbStart}ms`);
                } else {
                  console.log(`[Migration]   Thumbnail generation returned null (skipping)`);
                }
              } catch (thumbErr: any) {
                console.warn(`[Migration]   Thumbnail generation/upload failed (continuing without): ${thumbErr.message}`, thumbErr);
              }

              // Step 3: Build and encrypt photo payload
              const serverBlobType = blobTypeFromMime(photo.mime_type);
              const photoPayload = JSON.stringify({
                v: 1,
                filename: photo.filename,
                taken_at: photo.taken_at || photo.created_at,
                mime_type: photo.mime_type,
                media_type: (photo.media_type || mediaTypeFromMime(photo.mime_type)) as "photo" | "gif" | "video",
                width: photo.width,
                height: photo.height,
                duration: photo.duration_secs ?? undefined,
                latitude: photo.latitude ?? undefined,
                longitude: photo.longitude ?? undefined,
                album_ids: [],
                thumbnail_blob_id: thumbBlobId || "",
                data: arrayBufferToBase64(fileData),
              });

              const payloadBytes = new TextEncoder().encode(photoPayload);
              console.log(`[Migration]   Payload: ${(payloadBytes.length / 1024).toFixed(0)} KB (base64 inflated from ${(fileData.length / 1024).toFixed(0)} KB raw)`);

              const encStart = Date.now();
              const encPhoto = await encrypt(payloadBytes);
              console.log(`[Migration]   Encrypted: ${encPhoto.byteLength} bytes in ${Date.now() - encStart}ms`);

              const photoHash = await sha256Hex(new Uint8Array(encPhoto));

              // Step 4: Upload encrypted blob
              const uploadStart = Date.now();
              console.log(`[Migration]   Uploading encrypted photo (${encPhoto.byteLength} bytes, type=${serverBlobType}, hash=${photoHash.substring(0, 12)}...)...`);
              const uploadResult = await api.blobs.upload(encPhoto, serverBlobType, photoHash);
              console.log(`[Migration]   Upload complete in ${Date.now() - uploadStart}ms (total: ${Date.now() - stepStart}ms)`);

              // Step 5: Link the blob to the plain photo so it won't be re-migrated
              await api.photos.markEncrypted(photo.id, uploadResult.blob_id);
              console.log(`[Migration]   Linked photo ${photo.id} → blob ${uploadResult.blob_id}`);

              itemSuccess = true;
              succeeded++;
            } catch (itemErr: any) {
              console.error(`[Migration]   FAILED "${photo.filename}" attempt ${attempts}/3:`, itemErr.message, itemErr.stack);
              if (attempts >= 3) {
                failedCount++;
                lastError = `Failed on "${photo.filename}": ${itemErr.message}`;
                console.error(`[Migration]   GIVING UP on "${photo.filename}" after 3 attempts. Error: ${itemErr.message}`);
              }
              // Brief pause before retry to let transient issues settle
              if (attempts < 3) {
                const delay = 500 * attempts;
                console.log(`[Migration]   Retrying in ${delay}ms...`);
                await new Promise((r) => setTimeout(r, delay));
              }
            }
          }

          completed++;
          // Report progress (include error only when a photo exhausts all retries)
          if (!itemSuccess) {
            await api.encryption.reportProgress({
              completed_count: completed,
              error: lastError,
            });
          } else {
            await api.encryption.reportProgress({ completed_count: completed });
          }
        }

        console.log(`[Migration] Complete: ${succeeded} succeeded, ${failedCount} failed out of ${total}`);

        // Mark migration complete — report failures if any occurred
        if (failedCount > 0) {
          await api.encryption.reportProgress({
            completed_count: total,
            done: true,
            error: `Migration finished with ${failedCount}/${total} failures. Last error: ${lastError}`,
          });
        } else {
          await api.encryption.reportProgress({ completed_count: total, done: true });
        }
        await loadEncryptionSettings();
      } catch (err: any) {
        console.error("[Migration] Top-level migration error:", err.message, err.stack);
        await api.encryption.reportProgress({
          completed_count: 0,
          error: `Migration failed: ${err.message}`,
        }).catch(() => {});
      } finally {
        migrationRunningRef.current = false;
      }
    })();
  }, [migrationStatus, loadEncryptionSettings]);

  async function handleToggleEncryption() {
    setShowEncryptionWarning(false);
    setTogglingEncryption(true);
    setError("");
    try {
      const newMode = encryptionMode === "plain" ? "encrypted" : "plain";
      const res = await api.encryption.setMode(newMode);
      setEncryptionMode(newMode);
      setSuccess(res.message);
      // Reload to get migration status
      await loadEncryptionSettings();
    } catch (err: any) {
      setError(err.message);
    } finally {
      setTogglingEncryption(false);
    }
  }

  async function handleRecover() {
    if (backupServers.length === 0) return;
    setShowRecoverWarning(false);
    setRecovering(true);
    startTask("recovery");
    setError("");
    try {
      const target = backupServers.find((s) => s.enabled) ?? backupServers[0];
      const res = await api.backup.recover(target.id);
      setSuccess(res.message);
    } catch (err: any) {
      setError(err.message);
    } finally {
      setRecovering(false);
      endTask("recovery");
    }
  }

  async function handleAddBackupServer(e: React.FormEvent) {
    e.preventDefault();
    if (!backupServerName.trim() || !backupServerAddress.trim() || !backupServerApiKey.trim()) {
      setError("All backup server fields are required.");
      return;
    }
    const freq = parseInt(backupServerFrequency, 10);
    if (isNaN(freq) || freq < 1) {
      setError("Frequency must be a positive number of hours.");
      return;
    }
    setAddingBackupServer(true);
    setError("");
    try {
      await api.backup.addServer({
        name: backupServerName.trim(),
        address: backupServerAddress.trim(),
        api_key: backupServerApiKey.trim(),
        sync_frequency_hours: freq,
      });
      setSuccess("Backup server added successfully.");
      setShowAddBackupServer(false);
      setBackupServerName("");
      setBackupServerAddress("");
      setBackupServerApiKey("");
      setBackupServerFrequency("24");
      await loadBackupServers();
    } catch (err: any) {
      setError(err.message || "Failed to add backup server.");
    } finally {
      setAddingBackupServer(false);
    }
  }

  return (
    <div className="min-h-screen bg-gray-50 dark:bg-gray-900">
      <AppHeader />

      <main className="max-w-2xl mx-auto p-4">

      {error && (
        <p className="text-red-600 dark:text-red-400 text-sm mb-4 p-3 bg-red-50 dark:bg-red-900/30 rounded">{error}</p>
      )}
      {success && (
        <p className="text-green-600 dark:text-green-400 text-sm mb-4 p-3 bg-green-50 dark:bg-green-900/30 rounded">
          {success}
        </p>
      )}

      {/* ── Account ─────────────────────────────────────────────────────────── */}
      <section className="bg-white dark:bg-gray-800 rounded-lg shadow p-6 mb-4">
        <h2 className="text-lg font-semibold mb-3">Account</h2>
        <p className="text-gray-600 dark:text-gray-400">
          Signed in as <span className="font-medium">{username}</span>
        </p>
      </section>

      {/* ── Storage Usage ──────────────────────────────────────────────────── */}
      <StorageStatsSection stats={storageStats} loading={storageLoading} />

      {/* ── Server Selection ───────────────────────────────────────────────── */}
      {backupLoaded && (
        <section className="bg-white dark:bg-gray-800 rounded-lg shadow p-6 mb-4">
          <h2 className="text-lg font-semibold mb-3">Active Server</h2>
          <p className="text-sm text-gray-500 dark:text-gray-400 mb-3">
            Choose which server to view photos from.
          </p>
          <select
            value={viewMode === "main" ? "__main__" : (activeBackupServerId ?? "__main__")}
            onChange={(e) => {
              const val = e.target.value;
              if (val === "__main__") {
                setViewMode("main");
              } else {
                setActiveBackupServerId(val);
                setViewMode("backup");
              }
            }}
            className="w-full border rounded-md px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500 dark:bg-gray-700 dark:border-gray-600"
          >
            <option value="__main__">Main Server (local)</option>
            {backupServers.map((s) => (
              <option key={s.id} value={s.id}>
                {s.name} — {s.address}
              </option>
            ))}
          </select>
          {backupServers.length === 0 && (
            <p className="text-xs text-gray-400 mt-2">
              No backup servers configured. Add one in the Backup Recovery section below.
            </p>
          )}
        </section>
      )}

      {/* ── Scan for New Files (admin, plain mode) ───────────────────────── */}
      {isAdmin && encryptionMode === "plain" && !encryptionLoading && (
        <section className="bg-white dark:bg-gray-800 rounded-lg shadow p-6 mb-4">
          <h2 className="text-lg font-semibold mb-2">Scan for New Files</h2>
          <p className="text-sm text-gray-500 dark:text-gray-400 mb-3">
            Scan the storage directory for new photos and videos that haven't been registered yet.
          </p>
          <div className="flex items-center gap-3">
            <button
              onClick={async () => {
                setScanning(true);
                setScanResult(null);
                setError("");
                try {
                  const res = await api.admin.scanAndRegister();
                  setScanResult(
                    res.registered > 0
                      ? `Found and registered ${res.registered} new file${res.registered > 1 ? "s" : ""}.`
                      : "No new files found."
                  );
                } catch (err: any) {
                  setError(err.message || "Scan failed");
                } finally {
                  setScanning(false);
                }
              }}
              disabled={scanning}
              className="inline-flex items-center gap-1.5 bg-blue-600 text-white px-4 py-2 rounded-md hover:bg-blue-500 text-sm font-medium transition-colors disabled:opacity-50"
            >
              <AppIcon name="reload" size="w-4 h-4" className={scanning ? "animate-spin" : ""} />
              {scanning ? "Scanning…" : "Scan Now"}
            </button>
            {scanResult && (
              <span className="text-sm text-gray-600 dark:text-gray-400">{scanResult}</span>
            )}
          </div>
        </section>
      )}

      {/* ── Privacy & Encryption ─────────────────────────────────────────── */}
      <section className="bg-white dark:bg-gray-800 rounded-lg shadow p-6 mb-4">
        <h2 className="text-lg font-semibold mb-3">Privacy & Encryption</h2>

        {encryptionLoading ? (
          <div className="text-gray-400 text-sm">Loading encryption settings…</div>
        ) : (
          <div className="space-y-4">
            {/* Toggle switch */}
            <div className="flex items-center justify-between">
              <div>
                <h3 className="text-sm font-medium text-gray-700 dark:text-gray-300">
                  End-to-End Encryption
                </h3>
                <p className="text-sm text-gray-500 dark:text-gray-400">
                  {encryptionMode === "encrypted"
                    ? "Photos are encrypted — only you can view them."
                    : "Photos are stored as regular files on disk."}
                </p>
              </div>
              <button
                onClick={() => {
                  if (migrationStatus !== "idle") return;
                  setShowEncryptionWarning(true);
                }}
                disabled={togglingEncryption || migrationStatus !== "idle"}
                className={`relative inline-flex h-6 w-11 items-center rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-blue-500 focus:ring-offset-2 disabled:opacity-50 ${
                  encryptionMode === "encrypted" ? "bg-blue-600" : "bg-gray-300 dark:bg-gray-600"
                }`}
                role="switch"
                aria-checked={encryptionMode === "encrypted"}
              >
                <span
                  className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
                    encryptionMode === "encrypted" ? "translate-x-6" : "translate-x-1"
                  }`}
                />
              </button>
            </div>

            {/* Migration progress */}
            {(migrationStatus === "encrypting" || migrationStatus === "decrypting") && (() => {
              const pct = migrationTotal > 0 ? Math.min(Math.round((migrationCompleted / migrationTotal) * 100), 100) : 0;
              const action = migrationStatus === "encrypting" ? "Encrypting" : "Decrypting";
              return (
              <div className="bg-blue-50 dark:bg-blue-900/30 rounded-lg p-4">
                <div className="flex items-center gap-2 mb-2">
                  <div
                    className="w-4 h-4 border-2 border-blue-600 border-t-transparent rounded-full animate-spin cursor-help"
                    title={`${action} photos — ${pct}% complete (${migrationCompleted}/${migrationTotal})`}
                  />
                  <span
                    className="text-sm font-medium text-blue-700 dark:text-blue-300 cursor-help"
                    title={`${action} photos — ${pct}% complete (${migrationCompleted}/${migrationTotal})`}
                  >
                    {action} photos… {pct}%
                  </span>
                </div>
                <div className="w-full h-2 bg-blue-200 dark:bg-blue-800 rounded-full overflow-hidden">
                  <div
                    className="h-full bg-blue-600 rounded-full transition-all duration-500"
                    style={{ width: `${pct}%` }}
                  />
                </div>
                <p className="text-xs text-blue-600 dark:text-blue-400 mt-1">
                  {migrationCompleted} / {migrationTotal} items processed ({pct}%)
                </p>
              </div>
              );
            })()}

            {/* Migration error */}
            {migrationError && (
              <div className="bg-red-50 dark:bg-red-900/30 rounded-lg p-3">
                <p className="text-sm text-red-600 dark:text-red-400">
                  Migration error: {migrationError}
                </p>
              </div>
            )}

            {/* Toggle confirmation warning */}
            {showEncryptionWarning && (
              <div className="bg-amber-50 dark:bg-amber-900/20 border border-amber-200 dark:border-amber-800 rounded-lg p-4">
                <h4 className="text-sm font-semibold text-amber-800 dark:text-amber-300 mb-2">
                  ⚠️ {encryptionMode === "plain" ? "Enable Encryption?" : "Disable Encryption?"}
                </h4>
                <p className="text-sm text-amber-700 dark:text-amber-400 mb-3">
                  This process can take a significant amount of time depending on your library size.
                  It will run in the background — you can continue using the app while it processes.
                </p>
                <div className="flex gap-2">
                  <button
                    onClick={handleToggleEncryption}
                    disabled={togglingEncryption}
                    className={`px-4 py-2 rounded-md text-sm text-white disabled:opacity-50 ${
                      encryptionMode === "plain"
                        ? "bg-amber-600 hover:bg-amber-700"
                        : "bg-blue-600 hover:bg-blue-700"
                    }`}
                  >
                    {togglingEncryption ? "Switching…" : "Confirm"}
                  </button>
                  <button
                    onClick={() => setShowEncryptionWarning(false)}
                    className="bg-gray-200 dark:bg-gray-600 text-gray-800 dark:text-gray-200 px-4 py-2 rounded-md hover:bg-gray-300 dark:hover:bg-gray-500 text-sm"
                  >
                    Cancel
                  </button>
                </div>
              </div>
            )}
          </div>
        )}
      </section>

      {/* ── Backup Recovery ─────────────────────────────────────────────────── */}
      <section className="bg-white dark:bg-gray-800 rounded-lg shadow p-6 mb-4">
        <h2 className="text-lg font-semibold mb-3">Backup Recovery</h2>
        <p className="text-sm text-gray-500 dark:text-gray-400 mb-4">
          Recover photos from a configured backup server. Any photos on the backup
          that don't already exist on this server (by filename) will be downloaded and imported.
        </p>

        {!backupLoaded ? (
          <div className="text-gray-400 text-sm">Loading backup servers…</div>
        ) : backupServers.length === 0 ? (
          <div className="text-center py-4 border-2 border-dashed border-gray-200 dark:border-gray-600 rounded-lg">
            <p className="text-gray-400 text-sm">No backup servers configured.</p>
            <p className="text-xs text-gray-400 mt-1 mb-3">
              Auto-detection didn't find any servers. You can add one manually below.
            </p>
          </div>
        ) : !showRecoverWarning ? (
          <button
            onClick={() => {
              setShowRecoverWarning(true);
              setError("");
              setSuccess("");
            }}
            disabled={recovering}
            className="bg-amber-600 text-white px-4 py-2 rounded-md hover:bg-amber-700 disabled:opacity-50 text-sm"
          >
            {recovering ? (
              <span className="flex items-center gap-2">
                <span className="w-4 h-4 border-2 border-white border-t-transparent rounded-full animate-spin" />
                Recovering…
              </span>
            ) : (
              "Recover from Backup Server"
            )}
          </button>
        ) : (
          <div className="bg-amber-50 dark:bg-amber-900/20 border border-amber-200 dark:border-amber-800 rounded-lg p-4">
            <h4 className="text-sm font-semibold text-amber-800 dark:text-amber-300 mb-2">
              ⚠️ Confirm Recovery
            </h4>
            <p className="text-sm text-amber-700 dark:text-amber-400 mb-1">
              This will download <strong>all photos</strong> from the backup server
              {" "}<strong>"{backupServers.find((s) => s.enabled)?.name ?? backupServers[0]?.name}"</strong> to
              this server.
            </p>
            <ul className="text-sm text-amber-700 dark:text-amber-400 list-disc list-inside mb-3 space-y-0.5">
              <li>Photos with the same filename will be <strong>skipped</strong> (not overwritten).</li>
              <li>This process runs in the background and may take a while for large libraries.</li>
              <li>The backup server must be reachable and have its API key configured.</li>
            </ul>
            <div className="flex gap-2">
              <button
                onClick={handleRecover}
                disabled={recovering}
                className="bg-amber-600 text-white px-4 py-2 rounded-md hover:bg-amber-700 disabled:opacity-50 text-sm"
              >
                {recovering ? "Starting…" : "Confirm Recovery"}
              </button>
              <button
                onClick={() => setShowRecoverWarning(false)}
                className="bg-gray-200 dark:bg-gray-600 text-gray-800 dark:text-gray-200 px-4 py-2 rounded-md hover:bg-gray-300 dark:hover:bg-gray-500 text-sm"
              >
                Cancel
              </button>
            </div>
          </div>
        )}

        {/* ── Manually add a backup server ───────────────────────── */}
        <div className="mt-4 pt-4 border-t border-gray-200 dark:border-gray-700">
          {!showAddBackupServer ? (
            <button
              onClick={() => setShowAddBackupServer(true)}
              className="text-sm text-blue-600 dark:text-blue-400 hover:underline"
            >
              + Add backup server manually
            </button>
          ) : (
            <form onSubmit={handleAddBackupServer} className="space-y-3">
              <h4 className="text-sm font-semibold text-gray-700 dark:text-gray-300">Add Backup Server</h4>
              <div>
                <label className="block text-xs font-medium text-gray-600 dark:text-gray-400 mb-1">Name</label>
                <input
                  type="text"
                  value={backupServerName}
                  onChange={(e) => setBackupServerName(e.target.value)}
                  placeholder="My Backup Server"
                  className="w-full border rounded-md px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500 dark:bg-gray-700 dark:border-gray-600"
                />
              </div>
              <div>
                <label className="block text-xs font-medium text-gray-600 dark:text-gray-400 mb-1">Server Address</label>
                <input
                  type="text"
                  value={backupServerAddress}
                  onChange={(e) => setBackupServerAddress(e.target.value)}
                  placeholder="https://backup.example.com:8443"
                  className="w-full border rounded-md px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500 dark:bg-gray-700 dark:border-gray-600"
                />
              </div>
              <div>
                <label className="block text-xs font-medium text-gray-600 dark:text-gray-400 mb-1">API Key</label>
                <input
                  type="password"
                  value={backupServerApiKey}
                  onChange={(e) => setBackupServerApiKey(e.target.value)}
                  placeholder="Backup server API key"
                  className="w-full border rounded-md px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500 dark:bg-gray-700 dark:border-gray-600"
                />
              </div>
              <div>
                <label className="block text-xs font-medium text-gray-600 dark:text-gray-400 mb-1">Backup Frequency (hours)</label>
                <input
                  type="number"
                  min={1}
                  value={backupServerFrequency}
                  onChange={(e) => setBackupServerFrequency(e.target.value)}
                  className="w-28 border rounded-md px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500 dark:bg-gray-700 dark:border-gray-600"
                />
              </div>
              <div className="flex gap-2">
                <button
                  type="submit"
                  disabled={addingBackupServer}
                  className="bg-blue-600 text-white px-4 py-2 rounded-md hover:bg-blue-700 disabled:opacity-50 text-sm"
                >
                  {addingBackupServer ? (
                    <span className="flex items-center gap-2">
                      <span className="w-4 h-4 border-2 border-white border-t-transparent rounded-full animate-spin" />
                      Adding…
                    </span>
                  ) : (
                    "Add Server"
                  )}
                </button>
                <button
                  type="button"
                  onClick={() => setShowAddBackupServer(false)}
                  className="bg-gray-200 dark:bg-gray-600 text-gray-800 dark:text-gray-200 px-4 py-2 rounded-md hover:bg-gray-300 dark:hover:bg-gray-500 text-sm"
                >
                  Cancel
                </button>
              </div>
            </form>
          )}
        </div>
      </section>

      {/* ── Apps ───────────────────────────────────────────────────────────── */}
      <section className="bg-white dark:bg-gray-800 rounded-lg shadow p-6 mb-4">
        <h2 className="text-lg font-semibold mb-3">Apps</h2>
        <div className="space-y-4">
          <div>
            <h3 className="text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">Android App</h3>
            <p className="text-sm text-gray-500 dark:text-gray-400 mb-2">
              Download the Simple Photos Android app to automatically back up photos from your phone.
            </p>
            <button
              onClick={async () => {
                try {
                  const res = await fetch("/api/downloads/android", { method: "HEAD" });
                  if (res.ok) {
                    window.location.href = "/api/downloads/android";
                  } else {
                    setError("Android APK is not available yet. Build it with: cd android && ./gradlew assembleRelease — or place a pre-built APK at downloads/simple-photos.apk");
                  }
                } catch {
                  setError("Could not check APK availability.");
                }
              }}
              className="inline-flex items-center gap-1.5 bg-green-600 text-white px-4 py-2 rounded-md hover:bg-green-700 text-sm"
            >
              📱 Download Android App (.apk)
            </button>
          </div>
        </div>
      </section>

      {/* ── Change Password ─────────────────────────────────────────────────── */}
      <section className="bg-white dark:bg-gray-800 rounded-lg shadow p-6 mb-4">
        <h2 className="text-lg font-semibold mb-3">Password</h2>

        {!showChangePassword ? (
          <button
            onClick={() => {
              setShowChangePassword(true);
              setError("");
              setSuccess("");
            }}
            className="bg-blue-600 text-white px-4 py-2 rounded-md hover:bg-blue-700 text-sm"
          >
            Change Password
          </button>
        ) : (
          <form onSubmit={handleChangePassword} className="space-y-3">
            <div>
              <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
                Current Password
              </label>
              <input
                type="password"
                value={currentPassword}
                onChange={(e) => setCurrentPassword(e.target.value)}
                className="w-full border rounded-md px-3 py-2 focus:outline-none focus:ring-2 focus:ring-blue-500"
                required
                autoComplete="current-password"
                autoFocus
              />
            </div>

            <div>
              <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
                New Password
              </label>
              <input
                type="password"
                value={newPassword}
                onChange={(e) => setNewPassword(e.target.value)}
                className="w-full border rounded-md px-3 py-2 focus:outline-none focus:ring-2 focus:ring-blue-500"
                required
                minLength={8}
                maxLength={128}
                autoComplete="new-password"
              />
              {/* Strength bar */}
              {newPassword.length > 0 && (
                <div className="mt-2">
                  <div className="flex items-center gap-2 mb-1">
                    <div className="flex-1 h-1.5 bg-gray-200 dark:bg-gray-600 rounded-full overflow-hidden">
                      <div
                        className={`h-full rounded-full transition-all duration-300 ${pw.color}`}
                        style={{ width: `${(pw.score / pw.max) * 100}%` }}
                      />
                    </div>
                    <span className="text-xs font-medium text-gray-600 dark:text-gray-400 w-12 text-right">
                      {pw.label}
                    </span>
                  </div>
                  <ul className="text-xs space-y-0.5">
                    <li><Checkmark ok={pw.checks.length} /> At least 8 characters</li>
                    <li><Checkmark ok={pw.checks.uppercase} /> Uppercase letter</li>
                    <li><Checkmark ok={pw.checks.lowercase} /> Lowercase letter</li>
                    <li><Checkmark ok={pw.checks.digit} /> Number</li>
                  </ul>
                </div>
              )}
            </div>

            <div>
              <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
                Confirm New Password
              </label>
              <input
                type="password"
                value={confirmNewPassword}
                onChange={(e) => setConfirmNewPassword(e.target.value)}
                className="w-full border rounded-md px-3 py-2 focus:outline-none focus:ring-2 focus:ring-blue-500"
                required
                autoComplete="new-password"
              />
              {confirmNewPassword.length > 0 &&
                newPassword !== confirmNewPassword && (
                  <p className="text-xs text-red-500 dark:text-red-400 mt-1">
                    Passwords do not match
                  </p>
                )}
            </div>

            <div className="flex gap-2 pt-1">
              <button
                type="submit"
                disabled={loading || !pw.core}
                className="bg-blue-600 text-white px-4 py-2 rounded-md hover:bg-blue-700 disabled:opacity-50 text-sm"
              >
                {loading ? "Saving..." : "Update Password"}
              </button>
              <button
                type="button"
                onClick={() => {
                  setShowChangePassword(false);
                  setCurrentPassword("");
                  setNewPassword("");
                  setConfirmNewPassword("");
                }}
                className="bg-gray-200 dark:bg-gray-600 text-gray-800 dark:text-gray-200 px-4 py-2 rounded-md hover:bg-gray-300 dark:hover:bg-gray-500 text-sm"
              >
                Cancel
              </button>
            </div>
            <p className="text-xs text-gray-500 dark:text-gray-400">
              Changing your password will sign you out of all other sessions.
            </p>
          </form>
        )}
      </section>

      {/* ── Two-Factor Authentication ───────────────────────────────────────── */}
      <section className="bg-white dark:bg-gray-800 rounded-lg shadow p-6 mb-4">
        <h2 className="text-lg font-semibold mb-3">Two-Factor Authentication</h2>

        {!totpUri && !showDisable2fa && (
          <div className="flex gap-2">
            <button
              onClick={handleSetup2fa}
              disabled={loading}
              className="bg-blue-600 text-white px-4 py-2 rounded-md hover:bg-blue-700 text-sm disabled:opacity-50"
            >
              Enable 2FA
            </button>
            <button
              onClick={() => setShowDisable2fa(true)}
              className="bg-gray-200 dark:bg-gray-600 text-gray-800 dark:text-gray-200 px-4 py-2 rounded-md hover:bg-gray-300 dark:hover:bg-gray-500 text-sm"
            >
              Disable 2FA
            </button>
          </div>
        )}

        {totpUri && (
          <div className="space-y-4">
            <p className="text-sm text-gray-600 dark:text-gray-400">
              Scan this QR code with your authenticator app:
            </p>
            <div className="flex justify-center">
              <QRCodeSVG value={totpUri} size={200} />
            </div>

            {backupCodes.length > 0 && (
              <div>
                <p className="text-sm font-medium text-gray-700 dark:text-gray-300 mb-2">
                  Backup codes (save these somewhere safe):
                </p>
                <div className="bg-gray-100 dark:bg-gray-700 rounded p-3 font-mono text-sm grid grid-cols-2 gap-1">
                  {backupCodes.map((code, i) => (
                    <span key={i}>{code}</span>
                  ))}
                </div>
              </div>
            )}

            <form onSubmit={handleConfirm2fa} className="flex gap-2">
              <input
                type="text"
                value={totpCode}
                onChange={(e) => setTotpCode(e.target.value)}
                placeholder="Enter 6-digit code"
                className="flex-1 border rounded-md px-3 py-2 focus:outline-none focus:ring-2 focus:ring-blue-500"
                autoFocus
              />
              <button
                type="submit"
                disabled={loading}
                className="bg-blue-600 text-white px-4 py-2 rounded-md hover:bg-blue-700 disabled:opacity-50 text-sm"
              >
                Confirm
              </button>
            </form>
          </div>
        )}

        {showDisable2fa && (
          <form onSubmit={handleDisable2fa} className="space-y-3">
            <p className="text-sm text-gray-600 dark:text-gray-400">
              Enter a TOTP code to disable two-factor authentication:
            </p>
            <div className="flex gap-2">
              <input
                type="text"
                value={disableCode}
                onChange={(e) => setDisableCode(e.target.value)}
                placeholder="6-digit code"
                className="flex-1 border rounded-md px-3 py-2 focus:outline-none focus:ring-2 focus:ring-blue-500"
                autoFocus
              />
              <button
                type="submit"
                disabled={loading}
                className="bg-red-600 text-white px-4 py-2 rounded-md hover:bg-red-700 disabled:opacity-50 text-sm"
              >
                Disable
              </button>
              <button
                type="button"
                onClick={() => setShowDisable2fa(false)}
                className="bg-gray-200 dark:bg-gray-600 text-gray-800 dark:text-gray-200 px-4 py-2 rounded-md hover:bg-gray-300 dark:hover:bg-gray-500 text-sm"
              >
                Cancel
              </button>
            </div>
          </form>
        )}
      </section>

      {/* ── SSL / TLS (admin only) ─────────────────────────────────────────── */}
      {sslLoaded && (
        <section className="bg-white dark:bg-gray-800 rounded-lg shadow p-6 mb-4">
          <h2 className="text-lg font-semibold mb-3">SSL / TLS</h2>
          <p className="text-sm text-gray-500 dark:text-gray-400 mb-4">
            Serve your photos over HTTPS with a TLS certificate.
            Changes require a server restart.
          </p>

          {/* Enable toggle */}
          <div className="flex items-center justify-between mb-4">
            <div>
              <h3 className="text-sm font-medium text-gray-700 dark:text-gray-300">Enable TLS</h3>
              <p className="text-xs text-gray-500 dark:text-gray-400">
                {sslEnabled ? "HTTPS is enabled." : "Running on plain HTTP."}
              </p>
            </div>
            <button
              onClick={() => {
                setSslEnabled(!sslEnabled);
                setSslSaved(false);
              }}
              className={`relative inline-flex h-6 w-11 items-center rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-blue-500 focus:ring-offset-2 ${
                sslEnabled ? "bg-blue-600" : "bg-gray-300 dark:bg-gray-600"
              }`}
              role="switch"
              aria-checked={sslEnabled}
            >
              <span
                className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
                  sslEnabled ? "translate-x-6" : "translate-x-1"
                }`}
              />
            </button>
          </div>

          {/* Mode tabs */}
          {sslEnabled && (
            <div className="space-y-4">
              <div className="flex gap-2 mb-3">
                <button
                  onClick={() => setSslMode("manual")}
                  className={`px-3 py-1.5 rounded-md text-sm font-medium transition-colors ${
                    sslMode === "manual"
                      ? "bg-blue-600 text-white"
                      : "bg-gray-100 dark:bg-gray-700 text-gray-600 dark:text-gray-300 hover:bg-gray-200 dark:hover:bg-gray-600"
                  }`}
                >
                  Manual Certificate
                </button>
                <button
                  onClick={() => setSslMode("letsencrypt")}
                  className={`px-3 py-1.5 rounded-md text-sm font-medium transition-colors ${
                    sslMode === "letsencrypt"
                      ? "bg-green-600 text-white"
                      : "bg-gray-100 dark:bg-gray-700 text-gray-600 dark:text-gray-300 hover:bg-gray-200 dark:hover:bg-gray-600"
                  }`}
                >
                  Let's Encrypt
                </button>
              </div>

              {/* Manual cert fields */}
              {sslMode === "manual" && (
                <div className="space-y-3">
                  <div>
                    <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
                      Certificate Path
                    </label>
                    <input
                      type="text"
                      value={sslCertPath}
                      onChange={(e) => { setSslCertPath(e.target.value); setSslSaved(false); }}
                      placeholder="/etc/ssl/certs/my-cert.pem"
                      className="w-full border rounded-md px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
                    />
                  </div>
                  <div>
                    <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
                      Private Key Path
                    </label>
                    <input
                      type="text"
                      value={sslKeyPath}
                      onChange={(e) => { setSslKeyPath(e.target.value); setSslSaved(false); }}
                      placeholder="/etc/ssl/private/my-key.pem"
                      className="w-full border rounded-md px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
                    />
                  </div>
                  <button
                    onClick={handleSaveSsl}
                    disabled={sslSaving}
                    className="bg-blue-600 text-white px-4 py-2 rounded-md hover:bg-blue-700 disabled:opacity-50 text-sm"
                  >
                    {sslSaving ? "Saving…" : sslSaved ? "✓ Saved" : "Save TLS Configuration"}
                  </button>
                </div>
              )}

              {/* Let's Encrypt */}
              {sslMode === "letsencrypt" && !leGenerated && (
                <div className="space-y-3">
                  <div>
                    <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
                      Domain Name
                    </label>
                    <input
                      type="text"
                      value={leDomain}
                      onChange={(e) => setLeDomain(e.target.value)}
                      placeholder="photos.example.com"
                      className="w-full border rounded-md px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
                    />
                  </div>
                  <div>
                    <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
                      Contact E-mail
                    </label>
                    <input
                      type="email"
                      value={leEmail}
                      onChange={(e) => setLeEmail(e.target.value)}
                      placeholder="you@example.com"
                      className="w-full border rounded-md px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
                    />
                  </div>
                  <label className="flex items-center gap-2 text-sm text-gray-600 dark:text-gray-400">
                    <input
                      type="checkbox"
                      checked={leStaging}
                      onChange={(e) => setLeStaging(e.target.checked)}
                      className="accent-blue-600"
                    />
                    Use staging environment (testing only)
                  </label>
                  <div className="bg-amber-50 dark:bg-amber-900/20 border border-amber-200 dark:border-amber-800 rounded-lg p-3 text-xs text-amber-700 dark:text-amber-400">
                    Port 80 must be available and the domain must resolve to this server.
                  </div>
                  <button
                    onClick={handleGenerateLeCert}
                    disabled={leGenerating}
                    className="bg-green-600 text-white px-4 py-2 rounded-md hover:bg-green-700 disabled:opacity-50 text-sm"
                  >
                    {leGenerating ? (
                      <span className="flex items-center gap-2">
                        <span className="w-4 h-4 border-2 border-white border-t-transparent rounded-full animate-spin" />
                        Generating…
                      </span>
                    ) : (
                      "Generate Let's Encrypt Certificate"
                    )}
                  </button>

                  {/* Status log — real-time feedback during generation */}
                  {leStatusLog.length > 0 && (
                    <div className="mt-3 bg-gray-50 dark:bg-gray-900 border border-gray-200 dark:border-gray-700 rounded-lg p-3 max-h-48 overflow-y-auto font-mono text-xs space-y-0.5">
                      {leStatusLog.map((line, i) => (
                        <div
                          key={i}
                          className={
                            line.startsWith("ERROR")
                              ? "text-red-600 dark:text-red-400 font-semibold"
                              : line.includes("successfully")
                                ? "text-green-600 dark:text-green-400"
                                : "text-gray-600 dark:text-gray-400"
                          }
                        >
                          {line}
                        </div>
                      ))}
                    </div>
                  )}

                  {/* Persistent error banner */}
                  {leError && !leGenerating && (
                    <div className="mt-3 bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-800 rounded-lg p-3 text-sm text-red-700 dark:text-red-400">
                      <strong>Error:</strong> {leError}
                      <p className="text-xs mt-1 text-red-500 dark:text-red-500">
                        Common causes: port 80 blocked, domain doesn't resolve to this server, or rate limited by Let's Encrypt.
                      </p>
                    </div>
                  )}
                </div>
              )}

              {/* LE success */}
              {sslMode === "letsencrypt" && leGenerated && (
                <div className="bg-green-50 dark:bg-green-900/20 border border-green-200 dark:border-green-800 rounded-lg p-4 flex items-start gap-2">
                  <svg className="w-5 h-5 text-green-600 mt-0.5 flex-shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                    <path strokeLinecap="round" strokeLinejoin="round" d="M9 12.75L11.25 15 15 9.75M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
                  </svg>
                  <div>
                    <p className="text-sm font-medium text-green-700 dark:text-green-300">Certificate generated!</p>
                    <p className="text-xs text-green-600 dark:text-green-400 mt-1">
                      Restart the server to start serving HTTPS on {leDomain}.
                    </p>
                  </div>
                </div>
              )}
            </div>
          )}

          {/* Disable save btn */}
          {!sslEnabled && (
            <button
              onClick={handleSaveSsl}
              disabled={sslSaving}
              className="mt-2 bg-gray-600 text-white px-4 py-2 rounded-md hover:bg-gray-700 disabled:opacity-50 text-sm"
            >
              {sslSaving ? "Saving…" : "Disable TLS & Save"}
            </button>
          )}
        </section>
      )}

      {/* ── Manage Users (admin only) ────────────────────────────────────── */}
      {usersLoaded && isAdmin && (
        <section className="bg-white dark:bg-gray-800 rounded-lg shadow p-6 mb-4">
          <div className="flex items-center justify-between mb-4">
            <h2 className="text-lg font-semibold">Manage Users</h2>
            <button
              onClick={() => setShowAddUser(!showAddUser)}
              className="inline-flex items-center gap-1.5 bg-blue-600 text-white px-3 py-1.5 rounded-md hover:bg-blue-500 text-sm font-medium transition-colors"
            >
              <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M12 4.5v15m7.5-7.5h-15" />
              </svg>
              Add User
            </button>
          </div>

          {/* Add user form */}
          {showAddUser && (
            <form onSubmit={handleAddUser} className="mb-4 p-4 bg-gray-50 dark:bg-gray-700/50 rounded-lg space-y-3">
              <div>
                <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">Username</label>
                <input
                  type="text"
                  value={newUsername}
                  onChange={(e) => setNewUsername(e.target.value)}
                  className="w-full border rounded-md px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
                  required
                  minLength={3}
                  autoFocus
                />
              </div>
              <div>
                <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">Password</label>
                <input
                  type="password"
                  value={newUserPassword}
                  onChange={(e) => setNewUserPassword(e.target.value)}
                  className="w-full border rounded-md px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
                  required
                  minLength={8}
                />
              </div>
              <div>
                <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">Role</label>
                <div className="flex gap-4">
                  <label className="flex items-center gap-2 text-sm">
                    <input
                      type="radio"
                      checked={newUserRole === "user"}
                      onChange={() => setNewUserRole("user")}
                      className="accent-blue-600"
                    />
                    User
                  </label>
                  <label className="flex items-center gap-2 text-sm">
                    <input
                      type="radio"
                      checked={newUserRole === "admin"}
                      onChange={() => setNewUserRole("admin")}
                      className="accent-blue-600"
                    />
                    Admin
                  </label>
                </div>
              </div>
              <div className="flex gap-2">
                <button type="submit" className="bg-blue-600 text-white px-4 py-2 rounded-md hover:bg-blue-700 text-sm">
                  Create User
                </button>
                <button type="button" onClick={() => setShowAddUser(false)} className="px-4 py-2 rounded-md text-sm text-gray-600 dark:text-gray-400 hover:bg-gray-100 dark:hover:bg-gray-700">
                  Cancel
                </button>
              </div>
            </form>
          )}

          {/* User table */}
          <div className="overflow-x-auto">
            <table className="w-full text-sm">
              <thead>
                <tr className="border-b border-gray-200 dark:border-gray-700 text-left">
                  <th className="pb-2 font-medium text-gray-500 dark:text-gray-400">Username</th>
                  <th className="pb-2 font-medium text-gray-500 dark:text-gray-400">Role</th>
                  <th className="pb-2 font-medium text-gray-500 dark:text-gray-400">2FA</th>
                  <th className="pb-2 font-medium text-gray-500 dark:text-gray-400">Created</th>
                  <th className="pb-2 font-medium text-gray-500 dark:text-gray-400 text-right">Actions</th>
                </tr>
              </thead>
              <tbody>
                {managedUsers.map((u) => (
                  <tr key={u.id} className="border-b border-gray-100 dark:border-gray-700/50">
                    <td className="py-2.5 font-medium">{u.username}</td>
                    <td className="py-2.5">
                      {managedUsers.length > 1 && managedUsers.some(mu => mu.role === "admin") ? (
                        <select
                          value={u.role}
                          onChange={(e) => handleChangeRole(u.id, e.target.value as "admin" | "user")}
                          className="text-xs border rounded px-2 py-1 bg-transparent focus:outline-none focus:ring-1 focus:ring-blue-500"
                        >
                          <option value="user">User</option>
                          <option value="admin">Admin</option>
                        </select>
                      ) : (
                        <span className="text-xs capitalize text-gray-600 dark:text-gray-400">{u.role}</span>
                      )}
                    </td>
                    <td className="py-2.5">
                      {u.totp_enabled ? (
                        <span className="inline-flex items-center gap-1 text-green-600 dark:text-green-400 text-xs">
                          <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                            <path strokeLinecap="round" strokeLinejoin="round" d="M9 12.75L11.25 15 15 9.75M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
                          </svg>
                          Enabled
                        </span>
                      ) : (
                        <button
                          onClick={() => handleAdminSetup2fa(u.id)}
                          disabled={setup2faLoading}
                          className="text-xs text-blue-600 dark:text-blue-400 hover:text-blue-800 dark:hover:text-blue-300 font-medium transition-colors disabled:opacity-50"
                        >
                          Enable
                        </button>
                      )}
                    </td>
                    <td className="py-2.5 text-xs text-gray-500 dark:text-gray-400">
                      {new Date(u.created_at).toLocaleDateString()}
                    </td>
                    <td className="py-2.5 text-right">
                      <div className="flex items-center justify-end gap-1">
                        {/* Reset Password */}
                        <button
                          onClick={() => { setResetPwUserId(resetPwUserId === u.id ? null : u.id); setResetPwValue(""); }}
                          className="p-1.5 rounded hover:bg-gray-100 dark:hover:bg-gray-700 text-gray-500 dark:text-gray-400"
                          title="Reset password"
                        >
                          <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
                            <path strokeLinecap="round" strokeLinejoin="round" d="M15.75 5.25a3 3 0 013 3m3 0a6 6 0 01-7.029 5.912c-.563-.097-1.159.026-1.563.43L10.5 17.25H8.25v2.25H6v2.25H2.25v-2.818c0-.597.237-1.17.659-1.591l6.499-6.499c.404-.404.527-1 .43-1.563A6 6 0 1121.75 8.25z" />
                          </svg>
                        </button>
                        {/* Reset 2FA (only if enabled) */}
                        {u.totp_enabled && (
                          <button
                            onClick={() => handleResetUser2fa(u.id)}
                            className="p-1.5 rounded hover:bg-gray-100 dark:hover:bg-gray-700 text-gray-500 dark:text-gray-400"
                            title="Reset 2FA"
                          >
                            <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
                              <path strokeLinecap="round" strokeLinejoin="round" d="M16.5 10.5V6.75a4.5 4.5 0 10-9 0v3.75m-.75 11.25h10.5a2.25 2.25 0 002.25-2.25v-6.75a2.25 2.25 0 00-2.25-2.25H6.75a2.25 2.25 0 00-2.25 2.25v6.75a2.25 2.25 0 002.25 2.25z" />
                            </svg>
                          </button>
                        )}
                        {/* Delete */}
                        <button
                          onClick={() => setConfirmDeleteId(confirmDeleteId === u.id ? null : u.id)}
                          className="p-1.5 rounded hover:bg-red-50 dark:hover:bg-red-900/20 text-red-500"
                          title="Delete user"
                        >
                          <AppIcon name="trashcan" />
                        </button>
                      </div>
                      {/* Reset Password inline form */}
                      {resetPwUserId === u.id && (
                        <div className="flex gap-1 mt-2 justify-end">
                          <input
                            type="password"
                            value={resetPwValue}
                            onChange={(e) => setResetPwValue(e.target.value)}
                            placeholder="New password"
                            className="border rounded px-2 py-1 text-xs w-36 focus:outline-none focus:ring-1 focus:ring-blue-500"
                            autoFocus
                          />
                          <button
                            onClick={() => handleResetUserPassword(u.id)}
                            className="bg-blue-600 text-white px-2 py-1 rounded text-xs hover:bg-blue-700"
                          >
                            Set
                          </button>
                        </div>
                      )}
                      {/* Delete confirmation */}
                      {confirmDeleteId === u.id && (
                        <div className="flex items-center gap-1 mt-2 justify-end">
                          <span className="text-xs text-red-600 dark:text-red-400">Delete?</span>
                          <button
                            onClick={() => handleDeleteUser(u.id)}
                            className="bg-red-600 text-white px-2 py-1 rounded text-xs hover:bg-red-700"
                          >
                            Yes
                          </button>
                          <button
                            onClick={() => setConfirmDeleteId(null)}
                            className="px-2 py-1 rounded text-xs text-gray-500 hover:bg-gray-100 dark:hover:bg-gray-700"
                          >
                            No
                          </button>
                        </div>
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>

          {/* 2FA Setup Modal */}
          {setup2faUserId && setup2faUri && (
            <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-sm p-4">
              <div className="bg-white dark:bg-gray-800 rounded-xl shadow-2xl w-full max-w-md p-6">
                <h3 className="text-lg font-semibold text-gray-900 dark:text-gray-100 mb-2">
                  Enable 2FA for {managedUsers.find(u => u.id === setup2faUserId)?.username}
                </h3>
                <p className="text-sm text-gray-500 dark:text-gray-400 mb-4">
                  Scan this QR code with an authenticator app (Google Authenticator, Authy, etc.), then enter the 6-digit code to confirm.
                </p>

                <div className="flex justify-center mb-4">
                  <QRCodeSVG value={setup2faUri} size={200} />
                </div>

                <div className="mb-4">
                  <label className="block text-xs font-medium text-gray-600 dark:text-gray-400 mb-1">
                    Verification Code
                  </label>
                  <div className="flex gap-2">
                    <input
                      type="text"
                      value={setup2faCode}
                      onChange={(e) => setSetup2faCode(e.target.value.replace(/\D/g, "").slice(0, 6))}
                      onKeyDown={(e) => { if (e.key === "Enter") handleAdminConfirm2fa(); }}
                      placeholder="000000"
                      className="flex-1 border border-gray-300 dark:border-gray-600 rounded-md px-3 py-2 text-center font-mono text-lg tracking-widest focus:outline-none focus:ring-2 focus:ring-blue-500 dark:bg-gray-700"
                      maxLength={6}
                      autoFocus
                    />
                    <button
                      onClick={handleAdminConfirm2fa}
                      disabled={setup2faLoading || setup2faCode.length !== 6}
                      className="px-4 py-2 bg-blue-600 text-white rounded-md hover:bg-blue-700 disabled:opacity-50 text-sm font-medium transition-colors"
                    >
                      {setup2faLoading ? "Verifying…" : "Confirm"}
                    </button>
                  </div>
                </div>

                {setup2faBackupCodes.length > 0 && (
                  <details className="mb-4">
                    <summary className="text-xs text-gray-500 dark:text-gray-400 cursor-pointer hover:text-gray-700 dark:hover:text-gray-300">
                      Backup codes (save these!)
                    </summary>
                    <div className="mt-2 grid grid-cols-2 gap-1 p-3 bg-gray-50 dark:bg-gray-900 rounded-md font-mono text-xs">
                      {setup2faBackupCodes.map((code, i) => (
                        <span key={i} className="text-gray-700 dark:text-gray-300">{code}</span>
                      ))}
                    </div>
                  </details>
                )}

                <button
                  onClick={cancelAdminSetup2fa}
                  className="w-full mt-2 px-4 py-2 text-sm text-gray-600 dark:text-gray-400 hover:text-gray-800 dark:hover:text-gray-200 hover:bg-gray-100 dark:hover:bg-gray-700 rounded-md transition-colors"
                >
                  Cancel
                </button>
              </div>
            </div>
          )}
        </section>
      )}

      {/* ── About ───────────────────────────────────────────────────────────── */}
      <section className="bg-white dark:bg-gray-800 rounded-lg shadow p-6 mb-4">
        <h2 className="text-lg font-semibold mb-4">About</h2>
        <div className="flex flex-col items-center text-center">
          <img src="/logo.png" alt="Simple Photos" className="w-20 h-20 mb-3" />
          <h3 className="text-xl font-bold text-gray-900 dark:text-gray-100">Simple Photos</h3>
          <p className="text-sm text-gray-500 dark:text-gray-400 mb-4">
            v0.6.9 — Self-hosted, end-to-end encrypted photo & video library
          </p>
          <hr className="w-full border-gray-100 dark:border-gray-700 mb-4" />
          <p className="text-xs text-gray-400 mb-2">Developed by</p>
          <img
            src="/wulfnet.jpg"
            alt="WulfNet Designs"
            className="h-16 mb-1"
          />
          <p className="text-sm font-semibold text-gray-700 dark:text-gray-300">WulfNet Designs</p>
          <p className="text-xs text-gray-400 mt-3">
            &copy; {new Date().getFullYear()} WulfNet Designs. All rights
            reserved.
          </p>
        </div>
      </section>

      {/* ── Credits & Links ─────────────────────────────────────────────────── */}
      <section className="bg-white dark:bg-gray-800 rounded-lg shadow p-6 mb-4">
        <h2 className="text-lg font-semibold mb-4">Credits &amp; Links</h2>
        <div className="space-y-3 text-sm">
          <div className="flex items-center gap-3">
            <AppIcon name="star" size="w-5 h-5" />
            <div>
              <p className="text-gray-900 dark:text-gray-100 font-medium">Icons</p>
              <p className="text-gray-500 dark:text-gray-400">
                Custom icons by{" "}
                <a
                  href="https://www.flaticon.com/authors/angus-87"
                  target="_blank"
                  rel="noopener noreferrer"
                  className="text-blue-600 dark:text-blue-400 hover:underline"
                >
                  Angus_87
                </a>{" "}
                on{" "}
                <a
                  href="https://www.flaticon.com"
                  target="_blank"
                  rel="noopener noreferrer"
                  className="text-blue-600 dark:text-blue-400 hover:underline"
                >
                  Flaticon
                </a>
              </p>
            </div>
          </div>
          <hr className="border-gray-100 dark:border-gray-700" />
          <div className="flex items-center gap-3">
            <AppIcon name="shared" size="w-5 h-5" />
            <div>
              <p className="text-gray-900 dark:text-gray-100 font-medium">Source Code</p>
              <p className="text-gray-500 dark:text-gray-400">
                <a
                  href="https://github.com/wulfic/simple-photos"
                  target="_blank"
                  rel="noopener noreferrer"
                  className="text-blue-600 dark:text-blue-400 hover:underline"
                >
                  github.com/wulfic/simple-photos
                </a>
              </p>
            </div>
          </div>
        </div>
      </section>
      </main>
    </div>
  );
}

// ── Storage Stats Component ───────────────────────────────────────────────────

function formatBytes(bytes: number): string {
  if (bytes === 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.floor(Math.log(bytes) / Math.log(1024));
  const val = bytes / Math.pow(1024, i);
  return `${val < 10 ? val.toFixed(2) : val < 100 ? val.toFixed(1) : Math.round(val)} ${units[i]}`;
}

function StorageStatsSection({
  stats,
  loading,
}: {
  stats: {
    photo_bytes: number; photo_count: number;
    video_bytes: number; video_count: number;
    other_blob_bytes: number; other_blob_count: number;
    plain_bytes: number; plain_count: number;
    user_total_bytes: number;
    fs_total_bytes: number; fs_free_bytes: number;
  } | null;
  loading: boolean;
}) {
  if (loading) {
    return (
      <section className="bg-white dark:bg-gray-800 rounded-lg shadow p-6 mb-4">
        <h2 className="text-lg font-semibold mb-3">Storage</h2>
        <p className="text-sm text-gray-400 animate-pulse">Loading storage stats…</p>
      </section>
    );
  }

  if (!stats || stats.fs_total_bytes === 0) return null;

  const fsUsed = stats.fs_total_bytes - stats.fs_free_bytes;
  const otherUsage = Math.max(0, fsUsed - stats.user_total_bytes);

  // Percentages for the stacked bar
  const pctPhotos = (stats.photo_bytes + stats.plain_bytes) / stats.fs_total_bytes * 100;
  const pctVideos = stats.video_bytes / stats.fs_total_bytes * 100;
  const pctYou = stats.other_blob_bytes / stats.fs_total_bytes * 100;
  const pctOther = otherUsage / stats.fs_total_bytes * 100;
  const pctFree = stats.fs_free_bytes / stats.fs_total_bytes * 100;

  const totalPhotoCount = stats.photo_count + stats.plain_count;
  const totalPhotoBytes = stats.photo_bytes + stats.plain_bytes;

  return (
    <section className="bg-white dark:bg-gray-800 rounded-lg shadow p-6 mb-4">
      <h2 className="text-lg font-semibold mb-4">Storage</h2>

      {/* Stacked usage bar */}
      <div className="w-full h-5 rounded-full overflow-hidden flex bg-gray-200 dark:bg-gray-700 mb-3">
        {pctPhotos > 0 && (
          <div className="bg-blue-500 h-full transition-all" style={{ width: `${pctPhotos}%` }}
            title={`Photos: ${formatBytes(totalPhotoBytes)}`} />
        )}
        {pctVideos > 0 && (
          <div className="bg-purple-500 h-full transition-all" style={{ width: `${pctVideos}%` }}
            title={`Videos: ${formatBytes(stats.video_bytes)}`} />
        )}
        {pctYou > 0 && (
          <div className="bg-cyan-500 h-full transition-all" style={{ width: `${pctYou}%` }}
            title={`Other app data: ${formatBytes(stats.other_blob_bytes)}`} />
        )}
        {pctOther > 0 && (
          <div className="bg-gray-400 dark:bg-gray-500 h-full transition-all" style={{ width: `${pctOther}%` }}
            title={`System / other: ${formatBytes(otherUsage)}`} />
        )}
        {/* Free space is the remaining background (gray-200/700) */}
      </div>

      {/* Legend */}
      <div className="grid grid-cols-2 gap-x-6 gap-y-2 text-sm mb-4">
        <div className="flex items-center gap-2">
          <span className="w-3 h-3 rounded-full bg-blue-500 inline-block flex-shrink-0" />
          <span className="text-gray-600 dark:text-gray-400">Photos</span>
          <span className="ml-auto font-medium text-gray-900 dark:text-gray-100">
            {formatBytes(totalPhotoBytes)}
          </span>
        </div>
        <div className="flex items-center gap-2">
          <span className="w-3 h-3 rounded-full bg-purple-500 inline-block flex-shrink-0" />
          <span className="text-gray-600 dark:text-gray-400">Videos</span>
          <span className="ml-auto font-medium text-gray-900 dark:text-gray-100">
            {formatBytes(stats.video_bytes)}
          </span>
        </div>
        <div className="flex items-center gap-2">
          <span className="w-3 h-3 rounded-full bg-cyan-500 inline-block flex-shrink-0" />
          <span className="text-gray-600 dark:text-gray-400">App Data</span>
          <span className="ml-auto font-medium text-gray-900 dark:text-gray-100">
            {formatBytes(stats.other_blob_bytes)}
          </span>
        </div>
        <div className="flex items-center gap-2">
          <span className="w-3 h-3 rounded-full bg-gray-400 dark:bg-gray-500 inline-block flex-shrink-0" />
          <span className="text-gray-600 dark:text-gray-400">System / Other</span>
          <span className="ml-auto font-medium text-gray-900 dark:text-gray-100">
            {formatBytes(otherUsage)}
          </span>
        </div>
      </div>

      {/* Detail rows */}
      <div className="border-t border-gray-100 dark:border-gray-700 pt-3 space-y-1.5 text-sm">
        <div className="flex justify-between">
          <span className="text-gray-500 dark:text-gray-400">Your usage</span>
          <span className="font-medium text-gray-900 dark:text-gray-100">{formatBytes(stats.user_total_bytes)}</span>
        </div>
        <div className="flex justify-between text-xs">
          <span className="text-gray-400 dark:text-gray-500 pl-3">Photos &amp; GIFs ({totalPhotoCount})</span>
          <span className="text-gray-500 dark:text-gray-400">{formatBytes(totalPhotoBytes)}</span>
        </div>
        <div className="flex justify-between text-xs">
          <span className="text-gray-400 dark:text-gray-500 pl-3">Videos ({stats.video_count})</span>
          <span className="text-gray-500 dark:text-gray-400">{formatBytes(stats.video_bytes)}</span>
        </div>
        <div className="flex justify-between text-xs">
          <span className="text-gray-400 dark:text-gray-500 pl-3">Thumbnails &amp; manifests ({stats.other_blob_count})</span>
          <span className="text-gray-500 dark:text-gray-400">{formatBytes(stats.other_blob_bytes)}</span>
        </div>
        <div className="flex justify-between pt-1.5 border-t border-gray-100 dark:border-gray-700">
          <span className="text-gray-500 dark:text-gray-400">Free space</span>
          <span className="font-medium text-green-600 dark:text-green-400">{formatBytes(stats.fs_free_bytes)}</span>
        </div>
        <div className="flex justify-between">
          <span className="text-gray-500 dark:text-gray-400">Total capacity</span>
          <span className="font-medium text-gray-900 dark:text-gray-100">{formatBytes(stats.fs_total_bytes)}</span>
        </div>
      </div>
    </section>
  );
}