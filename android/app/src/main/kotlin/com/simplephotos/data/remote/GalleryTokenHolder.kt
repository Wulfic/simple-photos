package com.simplephotos.data.remote

/**
 * In-memory holder for the secure-gallery unlock token.
 *
 * When the user unlocks their secure albums (re-entering the account
 * password), the server returns a short-lived token. The server now requires
 * that token to serve secure-album media via the generic photo/blob endpoints,
 * so the OkHttp auth interceptor attaches it as the `X-Gallery-Token` header on
 * every request (harmless for non-secure endpoints, which the server ignores).
 *
 * Kept in memory only — session-scoped, cleared on process death and on logout
 * — which mirrors the web client's use of `sessionStorage` for the same token.
 */
object GalleryTokenHolder {
    @Volatile
    var token: String? = null
}
