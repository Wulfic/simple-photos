"""Diagnostic test to understand what's happening on the backup server 
with secure gallery items — checks blob_id, encrypted_thumb_blob_id,
and actual download availability."""

import time
import pytest
from helpers import APIClient, unique_filename
from conftest import USER_PASSWORD, TEST_ENCRYPTION_KEY
from cryptography.hazmat.primitives.ciphers.aead import AESGCM

NONCE_LENGTH = 12

def _aes_gcm_decrypt(key_hex, data):
    key = bytes.fromhex(key_hex)
    nonce = data[:NONCE_LENGTH]
    ciphertext = data[NONCE_LENGTH:]
    return AESGCM(key).decrypt(nonce, ciphertext, None)


class TestBackupSecureGalleryDiag:
    def test_backup_gallery_item_full_diagnostic(
        self, user_client, primary_admin, backup_server, backup_admin,
        backup_configured,
    ):
        """Full diagnostic: upload photo, add to secure gallery, sync,
        then check EVERYTHING on backup."""
        
        # Upload server-side photo and add to secure gallery
        photo = user_client.upload_photo(unique_filename())
        pid = photo["photo_id"]
        print(f"\n[DIAG] Uploaded photo: {pid}")

        gallery = user_client.create_secure_gallery("Diag Backup Test")
        gid = gallery["gallery_id"]
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        result = user_client.add_secure_gallery_item(gid, pid, token)
        clone_id = result["new_blob_id"]
        print(f"[DIAG] Clone blob_id: {clone_id}")

        # Wait for encryption migration
        print("[DIAG] Waiting 4s for encryption migration...")
        time.sleep(4)

        # Check primary state first
        print("\n=== PRIMARY STATE ===")
        items_primary = user_client.list_secure_gallery_items(gid, token)
        item_p = items_primary["items"][0]
        print(f"[DIAG] Primary list_gallery_items blob_id: {item_p['blob_id']}")
        print(f"[DIAG] Primary encrypted_thumb_blob_id: {item_p.get('encrypted_thumb_blob_id')}")
        print(f"[DIAG] Primary blob_id == clone_id? {item_p['blob_id'] == clone_id}")

        # Download photo blob on primary
        resp = user_client.download_blob(item_p["blob_id"])
        print(f"[DIAG] Primary photo blob download: HTTP {resp.status_code}, {len(resp.content)} bytes")
        
        # Download thumb blob on primary
        if item_p.get("encrypted_thumb_blob_id"):
            resp = user_client.download_blob(item_p["encrypted_thumb_blob_id"])
            print(f"[DIAG] Primary thumb blob download: HTTP {resp.status_code}, {len(resp.content)} bytes")

        # Trigger sync
        print("\n[DIAG] Triggering sync to backup...")
        primary_admin.admin_trigger_sync(backup_configured)
        time.sleep(8)  # Allow sync to complete

        # Now check backup
        print("\n=== BACKUP STATE ===")
        backup_user = APIClient(backup_server.base_url)
        backup_user.login(user_client.username, USER_PASSWORD)
        backup_token = backup_user.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]

        items_backup = backup_user.list_secure_gallery_items(gid, backup_token)
        assert len(items_backup["items"]) >= 1, "No items synced to backup"
        item_b = items_backup["items"][0]
        print(f"[DIAG] Backup list_gallery_items blob_id: {item_b['blob_id']}")
        print(f"[DIAG] Backup encrypted_thumb_blob_id: {item_b.get('encrypted_thumb_blob_id')}")
        
        # Try download photo blob on backup
        resp = backup_user.download_blob(item_b["blob_id"])
        print(f"[DIAG] Backup photo blob download: HTTP {resp.status_code}, {len(resp.content)} bytes")
        if resp.status_code == 200:
            try:
                pt = _aes_gcm_decrypt(TEST_ENCRYPTION_KEY, resp.content)
                print(f"[DIAG] Backup photo blob decrypt: OK, {len(pt)} bytes plaintext")
            except Exception as e:
                print(f"[DIAG] Backup photo blob decrypt FAILED: {e}")
        
        # Try download thumb blob on backup
        thumb_id = item_b.get("encrypted_thumb_blob_id")
        if thumb_id:
            resp = backup_user.download_blob(thumb_id)
            print(f"[DIAG] Backup thumb blob download: HTTP {resp.status_code}, {len(resp.content)} bytes")
            if resp.status_code == 200:
                try:
                    pt = _aes_gcm_decrypt(TEST_ENCRYPTION_KEY, resp.content)
                    print(f"[DIAG] Backup thumb blob decrypt: OK, {len(pt)} bytes plaintext")
                except Exception as e:
                    print(f"[DIAG] Backup thumb blob decrypt FAILED: {e}")
        else:
            print("[DIAG] No encrypted_thumb_blob_id in backup response!")

        # Check blobs listing on backup for diagnostic
        blobs = backup_user.list_blobs(limit=500)
        blob_ids = [b["id"] for b in blobs.get("blobs", [])]
        print(f"\n[DIAG] Backup blob count: {len(blob_ids)}")
        print(f"[DIAG] Photo blob_id in backup blobs list? {item_b['blob_id'] in blob_ids}")
        if thumb_id:
            print(f"[DIAG] Thumb blob_id in backup blobs list? {thumb_id in blob_ids}")

        # Also check: what does /api/blobs/{id} return on backup for
        # the photo blob (should be user-scoped)
        print(f"\n[DIAG] Checking if backup has the blob in its DB at all...")
        # Try downloading without auth to see if it's a 401 vs 404
        import requests
        raw_resp = requests.get(f"{backup_server.base_url}/api/blobs/{item_b['blob_id']}")
        print(f"[DIAG] Backup blob (no auth): HTTP {raw_resp.status_code}")
        
        # Final assertions to make the test report clearly
        assert resp.status_code != 404, (
            f"CONFIRMED BUG: Backup returns 404 for gallery item. "
            f"blob_id={item_b['blob_id']}, thumb_id={thumb_id}"
        )
