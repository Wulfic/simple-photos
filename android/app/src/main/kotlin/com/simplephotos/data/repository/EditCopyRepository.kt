/**
 * Edit-copy + render repository — non-destructive copies and server-side
 * render-and-download (PNG/JPEG output for an edited photo).
 */
package com.simplephotos.data.repository

import com.simplephotos.data.remote.ApiService
import com.simplephotos.data.remote.dto.*
import okhttp3.ResponseBody
import javax.inject.Inject
import javax.inject.Singleton

@Singleton
class EditCopyRepository @Inject constructor(private val api: ApiService) {

    suspend fun list(photoId: String): List<EditCopy> =
        api.listEditCopies(photoId).copies

    suspend fun create(
        photoId: String,
        editMetadata: String,
        name: String? = null,
    ): CreateEditCopyResponse =
        api.createEditCopy(photoId, CreateEditCopyRequest(name, editMetadata))

    suspend fun delete(photoId: String, copyId: String) {
        api.deleteEditCopy(photoId, copyId)
    }

    suspend fun render(
        photoId: String,
        editMetadata: String? = null,
        outputFormat: String? = null,
        quality: Int? = null,
    ): ResponseBody =
        api.renderPhoto(photoId, RenderPhotoRequest(editMetadata, outputFormat, quality))

    suspend fun sourceFile(photoId: String): ResponseBody = api.photoSourceFile(photoId)

    suspend fun webFile(photoId: String): ResponseBody = api.photoWebFile(photoId)
}
