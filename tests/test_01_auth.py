"""
Test 01: Authentication — register, login, 2FA, password change, token refresh, logout.
"""

import pytest
from helpers import APIClient, random_username, random_password


class TestSetupAndInit:
    """Server initialization and setup status."""

    def test_health_endpoint(self, primary_server):
        client = APIClient(primary_server.base_url)
        data = client.health()
        assert data["status"] == "ok"
        assert "version" in data

    def test_setup_status(self, primary_server, primary_admin):
        """After admin init, setup should report complete."""
        client = APIClient(primary_server.base_url)
        data = client.setup_status()
        assert data["setup_complete"] is True
        assert "version" in data

    def test_setup_init_rejects_when_already_setup(self, primary_server, primary_admin):
        """Re-init should fail once a user exists."""
        client = APIClient(primary_server.base_url)
        r = client.post("/api/setup/init", json_data={
            "username": "shouldfail", "password": "ShouldFail123!"
        })
        assert r.status_code in (400, 403, 409)


class TestRegistration:
    """User registration and validation."""

    def test_register_new_user(self, primary_server, primary_admin):
        client = APIClient(primary_server.base_url)
        username = random_username("reg_")
        r = client.post("/api/auth/register", json_data={
            "username": username, "password": "ValidPass123!"
        })
        assert r.status_code == 201
        data = r.json()
        assert data["username"] == username
        assert "user_id" in data

    def test_register_duplicate_username(self, primary_server, primary_admin):
        client = APIClient(primary_server.base_url)
        username = random_username("dup_")
        # First registration
        r = client.post("/api/auth/register", json_data={
            "username": username, "password": "ValidPass123!"
        })
        assert r.status_code == 201
        # Second registration with same username
        r = client.post("/api/auth/register", json_data={
            "username": username, "password": "ValidPass123!"
        })
        assert r.status_code == 409

    def test_register_weak_password(self, primary_server, primary_admin):
        client = APIClient(primary_server.base_url)
        r = client.post("/api/auth/register", json_data={
            "username": random_username(), "password": "short"
        })
        assert r.status_code == 400

    def test_register_invalid_username(self, primary_server, primary_admin):
        client = APIClient(primary_server.base_url)
        # Too short
        r = client.post("/api/auth/register", json_data={
            "username": "a", "password": "ValidPass123!"
        })
        assert r.status_code == 400


class TestLogin:
    """Login, token management, and logout."""

    def test_login_success(self, primary_server, primary_admin):
        username = random_username("login_")
        primary_admin.admin_create_user(username, "LoginPass123!")
        client = APIClient(primary_server.base_url)
        data = client.login(username, "LoginPass123!")
        assert "access_token" in data
        assert "refresh_token" in data
        assert "expires_in" in data

    def test_login_wrong_password(self, primary_server, primary_admin):
        username = random_username("badpw_")
        primary_admin.admin_create_user(username, "CorrectPass123!")
        client = APIClient(primary_server.base_url)
        r = client.post("/api/auth/login", json_data={
            "username": username, "password": "WrongPass123!"
        })
        assert r.status_code == 401

    def test_login_nonexistent_user(self, primary_server, primary_admin):
        client = APIClient(primary_server.base_url)
        r = client.post("/api/auth/login", json_data={
            "username": "doesnotexist99", "password": "Anything123!"
        })
        assert r.status_code == 401

    def test_token_refresh(self, user_client):
        old_access = user_client.access_token
        data = user_client.refresh()
        assert data["access_token"] != old_access
        assert "refresh_token" in data

    def test_refresh_token_rotation(self, user_client):
        """Old refresh token should be invalidated after rotation."""
        old_refresh = user_client.refresh_token
        user_client.refresh()
        # Old refresh token should now fail
        r = user_client.post("/api/auth/refresh", json_data={"refresh_token": old_refresh})
        assert r.status_code == 401

    def test_logout(self, primary_server, primary_admin):
        username = random_username("logout_")
        primary_admin.admin_create_user(username, "LogoutPass123!")
        client = APIClient(primary_server.base_url)
        client.login(username, "LogoutPass123!")
        assert client.access_token is not None

        refresh = client.refresh_token
        client.logout()

        # Refresh token should be revoked
        r = client.post("/api/auth/refresh", json_data={"refresh_token": refresh})
        assert r.status_code == 401

    def test_access_without_token(self, primary_server, primary_admin):
        client = APIClient(primary_server.base_url)
        r = client.get("/api/photos")
        assert r.status_code == 401


class TestPasswordChange:
    """Password change flow."""

    def test_change_password(self, primary_server, primary_admin):
        username = random_username("chpw_")
        old_pw = "OldPass123!"
        new_pw = "NewPass456!"
        primary_admin.admin_create_user(username, old_pw)

        client = APIClient(primary_server.base_url)
        client.login(username, old_pw)

        r = client.change_password(old_pw, new_pw)
        assert r.status_code == 200

        # Old password should fail
        client2 = APIClient(primary_server.base_url)
        r = client2.post("/api/auth/login", json_data={
            "username": username, "password": old_pw
        })
        assert r.status_code == 401

        # New password should work
        client2.login(username, new_pw)
        assert client2.access_token is not None

    def test_change_password_wrong_current(self, user_client):
        r = user_client.change_password("WrongCurrent123!", "NewPass456!")
        assert r.status_code in (400, 401, 403)

    def test_verify_password(self, primary_server, primary_admin):
        username = random_username("vpw_")
        pw = "VerifyPass123!"
        primary_admin.admin_create_user(username, pw)
        client = APIClient(primary_server.base_url)
        client.login(username, pw)

        r = client.post("/api/auth/verify-password", json_data={"password": pw})
        assert r.status_code == 200

        r = client.post("/api/auth/verify-password", json_data={"password": "Wrong123!"})
        assert r.status_code in (400, 401, 403)


class TestTwoFactorAuth:
    """2FA setup, confirm, login, and disable."""

    def test_2fa_initially_disabled(self, user_client):
        r = user_client.get("/api/auth/2fa/status")
        assert r.status_code == 200
        assert r.json()["totp_enabled"] is False

    def test_2fa_setup_returns_secret(self, user_client):
        r = user_client.post("/api/auth/2fa/setup")
        assert r.status_code == 200
        data = r.json()
        assert "otpauth_uri" in data
        assert "backup_codes" in data
        assert len(data["backup_codes"]) > 0


class TestAdminUserManagement:
    """Admin user CRUD operations."""

    def test_admin_create_user(self, admin_client):
        username = random_username("admcr_")
        data = admin_client.admin_create_user(username, "AdminCreate123!")
        assert data["username"] == username
        assert "user_id" in data

    def test_admin_list_users(self, admin_client):
        users = admin_client.admin_list_users()
        assert isinstance(users, list)
        assert len(users) >= 1  # At least the admin
        usernames = [u["username"] for u in users]
        assert "e2eadmin" in usernames

    def test_admin_delete_user(self, admin_client, primary_server):
        username = random_username("admdel_")
        data = admin_client.admin_create_user(username, "ToDelete123!")
        user_id = data["user_id"]

        r = admin_client.admin_delete_user(user_id)
        assert r.status_code == 204

        # Deleted user can't login
        client = APIClient(primary_server.base_url)
        r = client.post("/api/auth/login", json_data={
            "username": username, "password": "ToDelete123!"
        })
        assert r.status_code == 401

    def test_non_admin_cannot_manage_users(self, user_client):
        r = user_client.get("/api/admin/users")
        assert r.status_code == 403

        r = user_client.post("/api/admin/users", json_data={
            "username": "hacker", "password": "HackerPass123!"
        })
        assert r.status_code == 403
