/**
 * Slide-up photo Info panel — the Android port of the web
 * `web/src/components/viewer/PhotoInfoPanel.tsx`: a rich read-only "Photo
 * Details" view (dimensions, EXIF camera/exposure/location blocks, raw-EXIF
 * expander, write-to-file-EXIF) plus an inline "Edit Metadata" form (filename,
 * date, description, GPS, the full EXIF set, and a Photo Type dropdown that
 * also serves as the manual panorama/360 correction).
 *
 * Styling follows the shared token system ([SpViewer] always-dark viewer
 * palette + [SpButton]); the subtree forces [SpDarkColors] so recipes render
 * dark regardless of the app's light/dark setting, matching the web viewer.
 */
package com.simplephotos.ui.screens.viewer

import android.content.Intent
import android.net.Uri
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ArrowDropDown
import androidx.compose.material.icons.filled.Close
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.simplephotos.data.local.entities.PhotoEntity
import com.simplephotos.data.remote.dto.FullMetadataResponse
import com.simplephotos.data.remote.dto.MetadataUpdateRequest
import com.simplephotos.ui.components.SpButton
import com.simplephotos.ui.components.SpButtonVariant
import com.simplephotos.ui.theme.LocalSpColors
import com.simplephotos.ui.theme.SpDarkColors
import com.simplephotos.ui.theme.SpViewer

/** Photo types the user may assign by hand (mirrors the server allowlist in
 *  `metadata_edit.rs`); "none" is the sentinel for an ordinary photo. */
private val EDITABLE_SUBTYPES = listOf(
    "none" to "Normal",
    "panorama" to "Panorama",
    "equirectangular" to "360° Photo",
)

/** Collapse any stored subtype to the editable set (panorama/equirectangular
 *  map to themselves; everything else → "none"). The diff guard means an
 *  unchanged dropdown never overwrites a motion/burst/hdr value. */
private fun normalizeSubtype(raw: String?): String =
    if (raw == "panorama" || raw == "equirectangular") raw else "none"

@Composable
fun ViewerInfoPanel(
    visible: Boolean,
    photo: PhotoEntity?,
    fullMeta: FullMetadataResponse?,
    saving: Boolean,
    error: String?,
    onDismiss: () -> Unit,
    onSave: (request: MetadataUpdateRequest, newSubtype: String?) -> Unit,
    onWriteExif: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    val serverPhotoId = photo?.serverPhotoId

    var editing by remember { mutableStateOf(false) }
    var showExif by remember { mutableStateOf(false) }
    var validationError by remember { mutableStateOf<String?>(null) }
    // Set when we fire a save; the save-completed effect uses it to leave edit
    // mode only on success (mirrors the web `await save → setEditing(false)`).
    var awaitingSave by remember { mutableStateOf(false) }

    // ── Edit form state ──────────────────────────────────────────────────
    var editFilename by remember { mutableStateOf("") }
    var editTakenAt by remember { mutableStateOf("") }
    var editLat by remember { mutableStateOf("") }
    var editLon by remember { mutableStateOf("") }
    var editCamera by remember { mutableStateOf("") }
    var editSubtype by remember { mutableStateOf("none") }
    var editCameraMake by remember { mutableStateOf("") }
    var editLens by remember { mutableStateOf("") }
    var editIso by remember { mutableStateOf("") }
    var editFNumber by remember { mutableStateOf("") }
    var editExposureTime by remember { mutableStateOf("") }
    var editFocalLength by remember { mutableStateOf("") }
    var editFlash by remember { mutableStateOf("") }
    var editWhiteBalance by remember { mutableStateOf("") }
    var editExposureProgram by remember { mutableStateOf("") }
    var editMeteringMode by remember { mutableStateOf("") }
    var editOrientation by remember { mutableStateOf("") }
    var editSoftware by remember { mutableStateOf("") }
    var editArtist by remember { mutableStateOf("") }
    var editCopyright by remember { mutableStateOf("") }
    var editDescription by remember { mutableStateOf("") }
    var editUserComment by remember { mutableStateOf("") }
    var editColorSpace by remember { mutableStateOf("") }
    var editExposureBias by remember { mutableStateOf("") }
    var editSceneType by remember { mutableStateOf("") }
    var editDigitalZoom by remember { mutableStateOf("") }

    // Reset transient UI when the panel closes.
    LaunchedEffect(visible) {
        if (!visible) {
            editing = false
            showExif = false
            validationError = null
            awaitingSave = false
        }
    }

    // Leave edit mode once a fired save completes without error.
    LaunchedEffect(saving) {
        if (awaitingSave && !saving) {
            awaitingSave = false
            if (error == null) editing = false
        }
    }

    fun startEdit() {
        editFilename = photo?.filename ?: ""
        editTakenAt = fullMeta?.takenAt ?: ""
        editLat = photo?.latitude?.toString() ?: ""
        editLon = photo?.longitude?.toString() ?: ""
        editCamera = photo?.cameraModel ?: fullMeta?.cameraModel ?: ""
        editSubtype = normalizeSubtype(fullMeta?.photoSubtype ?: photo?.photoSubtype)
        editCameraMake = fullMeta?.cameraMake ?: ""
        editLens = fullMeta?.lensModel ?: ""
        editIso = fullMeta?.isoSpeed?.toString() ?: ""
        editFNumber = fullMeta?.fNumber?.toString() ?: ""
        editExposureTime = fullMeta?.exposureTime ?: ""
        editFocalLength = fullMeta?.focalLength?.toString() ?: ""
        editFlash = fullMeta?.flash ?: ""
        editWhiteBalance = fullMeta?.whiteBalance ?: ""
        editExposureProgram = fullMeta?.exposureProgram ?: ""
        editMeteringMode = fullMeta?.meteringMode ?: ""
        editOrientation = fullMeta?.orientation?.toString() ?: ""
        editSoftware = fullMeta?.software ?: ""
        editArtist = fullMeta?.artist ?: ""
        editCopyright = fullMeta?.copyright ?: ""
        editDescription = fullMeta?.description ?: ""
        editUserComment = fullMeta?.userComment ?: ""
        editColorSpace = fullMeta?.colorSpace ?: ""
        editExposureBias = fullMeta?.exposureBias?.toString() ?: ""
        editSceneType = fullMeta?.sceneType ?: ""
        editDigitalZoom = fullMeta?.digitalZoom?.toString() ?: ""
        validationError = null
        editing = true
    }

    AnimatedVisibility(
        visible = visible,
        enter = slideInVertically { it },
        exit = slideOutVertically { it },
        modifier = modifier,
    ) {
        // Force the always-dark palette so SpButton (which reads LocalSpColors)
        // renders dark even when the app is in light mode.
        CompositionLocalProvider(LocalSpColors provides SpDarkColors) {
            val maxPanelHeight = (LocalConfiguration.current.screenHeightDp * 0.62f).dp
            Surface(
                color = SpViewer.panelBg,
                shape = RoundedCornerShape(topStart = 16.dp, topEnd = 16.dp),
                modifier = Modifier.fillMaxWidth(),
            ) {
                Column(
                    modifier = Modifier
                        .fillMaxWidth()
                        .navigationBarsPadding()
                        .heightIn(max = maxPanelHeight),
                ) {
                    // ── Header ──
                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .padding(start = 20.dp, end = 12.dp, top = 12.dp, bottom = 12.dp),
                        horizontalArrangement = Arrangement.SpaceBetween,
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Text(
                            if (editing) "Edit Metadata" else "Photo Details",
                            color = SpViewer.textPrimary,
                            fontWeight = FontWeight.SemiBold,
                            fontSize = 14.sp,
                        )
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            if (!editing && serverPhotoId != null) {
                                Text(
                                    "Edit",
                                    color = SpViewer.accent,
                                    fontSize = 13.sp,
                                    fontWeight = FontWeight.Medium,
                                    modifier = Modifier
                                        .clip(RoundedCornerShape(6.dp))
                                        .clickable { startEdit() }
                                        .padding(horizontal = 10.dp, vertical = 4.dp),
                                )
                            }
                            IconButton(onClick = onDismiss, modifier = Modifier.size(32.dp)) {
                                Icon(
                                    Icons.Default.Close,
                                    contentDescription = "Close",
                                    tint = SpViewer.textMuted,
                                    modifier = Modifier.size(18.dp),
                                )
                            }
                        }
                    }

                    HorizontalDividerThin()

                    Column(
                        modifier = Modifier
                            .fillMaxWidth()
                            // Hard-bound the scroll body to the panel height minus the
                            // header so verticalScroll gets a real (overflowing) viewport
                            // and can actually scroll. A non-weighted scroll child in a
                            // height-capped Column is otherwise measured at full content
                            // height (viewport == content → scroll range 0), so the long
                            // edit form was clipped with the lower fields and Save/Cancel
                            // unreachable. heightIn(max) still lets short (view-mode)
                            // content wrap instead of forcing full panel height.
                            .heightIn(max = maxPanelHeight - 64.dp)
                            .verticalScroll(rememberScrollState())
                            .padding(horizontal = 20.dp, vertical = 14.dp),
                        verticalArrangement = Arrangement.spacedBy(6.dp),
                    ) {
                        val shownError = validationError ?: error
                        if (shownError != null) {
                            Box(
                                modifier = Modifier
                                    .fillMaxWidth()
                                    .clip(RoundedCornerShape(6.dp))
                                    .background(SpViewer.dangerBg)
                                    .padding(horizontal = 10.dp, vertical = 6.dp),
                            ) {
                                Text(shownError, color = SpViewer.dangerText, fontSize = 12.sp)
                            }
                        }

                        if (photo == null) {
                            Text(
                                "No metadata available",
                                color = SpViewer.textMuted,
                                fontSize = 13.sp,
                                fontStyle = androidx.compose.ui.text.font.FontStyle.Italic,
                            )
                        } else if (editing) {
                            EditModeContent(
                                editFilename = editFilename, onFilename = { editFilename = it },
                                editTakenAt = editTakenAt, onTakenAt = { editTakenAt = it },
                                editDescription = editDescription, onDescription = { editDescription = it },
                                editUserComment = editUserComment, onUserComment = { editUserComment = it },
                                editSubtype = editSubtype, onSubtype = { editSubtype = it },
                                editLat = editLat, onLat = { editLat = it },
                                editLon = editLon, onLon = { editLon = it },
                                editCamera = editCamera, onCamera = { editCamera = it },
                                editCameraMake = editCameraMake, onCameraMake = { editCameraMake = it },
                                editLens = editLens, onLens = { editLens = it },
                                editIso = editIso, onIso = { editIso = it },
                                editFNumber = editFNumber, onFNumber = { editFNumber = it },
                                editExposureTime = editExposureTime, onExposureTime = { editExposureTime = it },
                                editFocalLength = editFocalLength, onFocalLength = { editFocalLength = it },
                                editExposureBias = editExposureBias, onExposureBias = { editExposureBias = it },
                                editExposureProgram = editExposureProgram, onExposureProgram = { editExposureProgram = it },
                                editMeteringMode = editMeteringMode, onMeteringMode = { editMeteringMode = it },
                                editFlash = editFlash, onFlash = { editFlash = it },
                                editWhiteBalance = editWhiteBalance, onWhiteBalance = { editWhiteBalance = it },
                                editOrientation = editOrientation, onOrientation = { editOrientation = it },
                                editColorSpace = editColorSpace, onColorSpace = { editColorSpace = it },
                                editSceneType = editSceneType, onSceneType = { editSceneType = it },
                                editDigitalZoom = editDigitalZoom, onDigitalZoom = { editDigitalZoom = it },
                                editSoftware = editSoftware, onSoftware = { editSoftware = it },
                                editArtist = editArtist, onArtist = { editArtist = it },
                                editCopyright = editCopyright, onCopyright = { editCopyright = it },
                                saving = saving,
                                onCancel = { editing = false; validationError = null },
                                onSaveClick = onSaveClick@{
                                    validationError = null
                                    // ── GPS validation (mirrors web) ──
                                    val hadGps = photo.latitude != null
                                    val hasGps = editLat.isNotBlank() && editLon.isNotBlank()
                                    var latOut: Double? = null
                                    var lonOut: Double? = null
                                    var clearGpsOut: Boolean? = null
                                    if (hadGps && !hasGps) {
                                        clearGpsOut = true
                                    } else if (hasGps) {
                                        val lat = editLat.toDoubleOrNull()
                                        val lon = editLon.toDoubleOrNull()
                                        if (lat == null || lon == null) {
                                            validationError = "Invalid coordinate values"; return@onSaveClick
                                        }
                                        if (lat < -90 || lat > 90) {
                                            validationError = "Latitude must be between -90 and 90"; return@onSaveClick
                                        }
                                        if (lon < -180 || lon > 180) {
                                            validationError = "Longitude must be between -180 and 180"; return@onSaveClick
                                        }
                                        if (lat != photo.latitude || lon != photo.longitude) {
                                            latOut = lat; lonOut = lon
                                        }
                                    }

                                    val diffStr = { edit: String, orig: String? ->
                                        if (edit != (orig ?: "")) edit.ifBlank { null } else null
                                    }
                                    val diffInt = { edit: String, orig: Int? ->
                                        if (edit != (orig?.toString() ?: "") && edit.isNotBlank()) edit.toIntOrNull() else null
                                    }
                                    val diffDbl = { edit: String, orig: Double? ->
                                        if (edit != (orig?.toString() ?: "") && edit.isNotBlank()) edit.toDoubleOrNull() else null
                                    }
                                    val subtypeChanged = editSubtype != normalizeSubtype(fullMeta?.photoSubtype ?: photo.photoSubtype)

                                    val req = MetadataUpdateRequest(
                                        filename = diffStr(editFilename, photo.filename),
                                        takenAt = diffStr(editTakenAt, fullMeta?.takenAt),
                                        cameraModel = diffStr(editCamera, photo.cameraModel ?: fullMeta?.cameraModel),
                                        photoSubtype = if (subtypeChanged) editSubtype else null,
                                        latitude = latOut,
                                        longitude = lonOut,
                                        clearGps = clearGpsOut,
                                        cameraMake = diffStr(editCameraMake, fullMeta?.cameraMake),
                                        lensModel = diffStr(editLens, fullMeta?.lensModel),
                                        isoSpeed = diffInt(editIso, fullMeta?.isoSpeed),
                                        fNumber = diffDbl(editFNumber, fullMeta?.fNumber),
                                        exposureTime = diffStr(editExposureTime, fullMeta?.exposureTime),
                                        focalLength = diffDbl(editFocalLength, fullMeta?.focalLength),
                                        flash = diffStr(editFlash, fullMeta?.flash),
                                        whiteBalance = diffStr(editWhiteBalance, fullMeta?.whiteBalance),
                                        exposureProgram = diffStr(editExposureProgram, fullMeta?.exposureProgram),
                                        meteringMode = diffStr(editMeteringMode, fullMeta?.meteringMode),
                                        orientation = diffInt(editOrientation, fullMeta?.orientation),
                                        software = diffStr(editSoftware, fullMeta?.software),
                                        artist = diffStr(editArtist, fullMeta?.artist),
                                        copyright = diffStr(editCopyright, fullMeta?.copyright),
                                        description = diffStr(editDescription, fullMeta?.description),
                                        userComment = diffStr(editUserComment, fullMeta?.userComment),
                                        colorSpace = diffStr(editColorSpace, fullMeta?.colorSpace),
                                        exposureBias = diffDbl(editExposureBias, fullMeta?.exposureBias),
                                        sceneType = diffStr(editSceneType, fullMeta?.sceneType),
                                        digitalZoom = diffDbl(editDigitalZoom, fullMeta?.digitalZoom),
                                    )

                                    if (req == MetadataUpdateRequest()) {
                                        editing = false   // nothing changed
                                        return@onSaveClick
                                    }
                                    awaitingSave = true
                                    onSave(req, if (subtypeChanged) editSubtype else null)
                                },
                            )
                        } else {
                            ViewModeContent(
                                photo = photo,
                                fullMeta = fullMeta,
                                saving = saving,
                                onWriteExif = onWriteExif,
                                showExif = showExif,
                                onToggleExif = { showExif = !showExif },
                                onOpenMap = { lat, lon ->
                                    try {
                                        context.startActivity(
                                            Intent(
                                                Intent.ACTION_VIEW,
                                                Uri.parse("https://www.google.com/maps?q=$lat,$lon"),
                                            ),
                                        )
                                    } catch (_: Exception) { /* no map app */ }
                                },
                            )
                        }
                    }
                }
            }
        }
    }
}

// ── Read-only "Photo Details" view ───────────────────────────────────────────

@Composable
private fun ViewModeContent(
    photo: PhotoEntity,
    fullMeta: FullMetadataResponse?,
    saving: Boolean,
    onWriteExif: () -> Unit,
    showExif: Boolean,
    onToggleExif: () -> Unit,
    onOpenMap: (Double, Double) -> Unit,
) {
    val dateFmt = remember { java.text.SimpleDateFormat("MMM d, yyyy  h:mm a", java.util.Locale.getDefault()) }

    InfoRow("Filename", photo.filename)
    InfoRow("Type", photo.mimeType)
    if (photo.width > 0 && photo.height > 0) InfoRow("Dimensions", "${photo.width} × ${photo.height}")
    photo.sizeBytes?.let { if (it > 0) InfoRow("Size", formatInfoBytes(it)) }
    if (photo.takenAt > 0L) InfoRow("Taken", dateFmt.format(java.util.Date(photo.takenAt)))
    if (photo.createdAt > 0L) InfoRow("Uploaded", dateFmt.format(java.util.Date(photo.createdAt)))
    photo.durationSecs?.let { InfoRow("Duration", "%.1fs".format(it)) }
    fullMeta?.description?.let { if (it.isNotBlank()) InfoRow("Description", it) }

    // Camera / Lens
    val cameraModel = photo.cameraModel ?: fullMeta?.cameraModel
    if (cameraModel != null || fullMeta?.cameraMake != null || fullMeta?.lensModel != null) {
        SectionDivider()
        if (cameraModel != null) {
            val make = fullMeta?.cameraMake
            InfoRow("Camera", if (make != null) "$make $cameraModel" else cameraModel)
        }
        fullMeta?.lensModel?.let { InfoRow("Lens", it) }
    }

    // Exposure
    if (fullMeta?.isoSpeed != null || fullMeta?.fNumber != null ||
        fullMeta?.exposureTime != null || fullMeta?.focalLength != null
    ) {
        SectionDivider()
        fullMeta.isoSpeed?.let { InfoRow("ISO", it.toString()) }
        fullMeta.fNumber?.let { InfoRow("Aperture", "f/$it") }
        fullMeta.exposureTime?.let { InfoRow("Shutter", it) }
        fullMeta.focalLength?.let { InfoRow("Focal Length", "${it}mm") }
        fullMeta.flash?.let { InfoRow("Flash", it) }
        fullMeta.whiteBalance?.let { InfoRow("White Balance", it) }
        fullMeta.meteringMode?.let { InfoRow("Metering", it) }
    }

    // Location
    fullMeta?.geoCity?.let { city ->
        val parts = listOfNotNull(city, fullMeta.geoState, fullMeta.geoCountry).filter { it.isNotBlank() }
        if (parts.isNotEmpty()) InfoRow("Location", parts.joinToString(", "))
    }
    val lat = photo.latitude
    val lon = photo.longitude
    if (lat != null && lon != null) {
        Row(
            modifier = Modifier.fillMaxWidth().padding(vertical = 3.dp),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.Top,
        ) {
            Text("GPS", color = SpViewer.textMuted, fontSize = 13.sp)
            Text(
                "%.5f, %.5f ↗".format(lat, lon),
                color = SpViewer.accent,
                fontSize = 13.sp,
                textAlign = TextAlign.End,
                modifier = Modifier.clickable { onOpenMap(lat, lon) },
            )
        }
    }

    // Other
    if (fullMeta?.artist != null || fullMeta?.copyright != null || fullMeta?.software != null) {
        SectionDivider()
        fullMeta.artist?.let { InfoRow("Artist", it) }
        fullMeta.copyright?.let { InfoRow("Copyright", it) }
        fullMeta.software?.let { InfoRow("Software", it) }
    }

    // Raw EXIF expander
    val exif = fullMeta?.exifTags
    if (!exif.isNullOrEmpty()) {
        SectionDivider()
        Text(
            "${if (showExif) "▼" else "▶"} Raw EXIF (${exif.size} tags)",
            color = SpViewer.textMuted,
            fontSize = 12.sp,
            modifier = Modifier.fillMaxWidth().clickable { onToggleExif() }.padding(vertical = 2.dp),
        )
        if (showExif) {
            Column(
                modifier = Modifier
                    .fillMaxWidth()
                    .heightIn(max = 200.dp)
                    .verticalScroll(rememberScrollState())
                    .padding(top = 4.dp),
                verticalArrangement = Arrangement.spacedBy(3.dp),
            ) {
                exif.toSortedMap().forEach { (k, v) ->
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.SpaceBetween,
                    ) {
                        Text(k, color = SpViewer.textSubtle, fontSize = 11.sp, modifier = Modifier.weight(1f))
                        Text(
                            v,
                            color = SpViewer.textFaint,
                            fontSize = 11.sp,
                            maxLines = 2,
                            textAlign = TextAlign.End,
                            modifier = Modifier.weight(1.4f),
                        )
                    }
                }
            }
        }
    }

    // Write to file EXIF (jpeg/tiff only)
    val mime = photo.mimeType.lowercase()
    val isJpegOrTiff = mime.contains("jpeg") || mime.contains("jpg") || mime.contains("tiff")
    if (isJpegOrTiff && photo.serverPhotoId != null) {
        SectionDivider()
        Text(
            if (saving) "Writing..." else "Write to File EXIF",
            color = if (saving) SpViewer.textSubtle else SpViewer.textMuted,
            fontSize = 12.sp,
            modifier = Modifier
                .clickable(enabled = !saving) { onWriteExif() }
                .padding(vertical = 2.dp),
        )
    }
}

// ── Inline "Edit Metadata" form ──────────────────────────────────────────────

@Composable
private fun EditModeContent(
    editFilename: String, onFilename: (String) -> Unit,
    editTakenAt: String, onTakenAt: (String) -> Unit,
    editDescription: String, onDescription: (String) -> Unit,
    editUserComment: String, onUserComment: (String) -> Unit,
    editSubtype: String, onSubtype: (String) -> Unit,
    editLat: String, onLat: (String) -> Unit,
    editLon: String, onLon: (String) -> Unit,
    editCamera: String, onCamera: (String) -> Unit,
    editCameraMake: String, onCameraMake: (String) -> Unit,
    editLens: String, onLens: (String) -> Unit,
    editIso: String, onIso: (String) -> Unit,
    editFNumber: String, onFNumber: (String) -> Unit,
    editExposureTime: String, onExposureTime: (String) -> Unit,
    editFocalLength: String, onFocalLength: (String) -> Unit,
    editExposureBias: String, onExposureBias: (String) -> Unit,
    editExposureProgram: String, onExposureProgram: (String) -> Unit,
    editMeteringMode: String, onMeteringMode: (String) -> Unit,
    editFlash: String, onFlash: (String) -> Unit,
    editWhiteBalance: String, onWhiteBalance: (String) -> Unit,
    editOrientation: String, onOrientation: (String) -> Unit,
    editColorSpace: String, onColorSpace: (String) -> Unit,
    editSceneType: String, onSceneType: (String) -> Unit,
    editDigitalZoom: String, onDigitalZoom: (String) -> Unit,
    editSoftware: String, onSoftware: (String) -> Unit,
    editArtist: String, onArtist: (String) -> Unit,
    editCopyright: String, onCopyright: (String) -> Unit,
    saving: Boolean,
    onCancel: () -> Unit,
    onSaveClick: () -> Unit,
) {
    SectionLabel("File")
    EditRow("Filename", editFilename, onFilename)
    EditRow("Date Taken", editTakenAt, onTakenAt, placeholder = "2024-01-15T14:30:00Z")
    EditRow("Description", editDescription, onDescription)
    EditRow("Comment", editUserComment, onUserComment)
    DropdownRow("Photo Type", editSubtype, EDITABLE_SUBTYPES, onSubtype)

    SectionLabel("Location")
    EditRow("Latitude", editLat, onLat, placeholder = "-90 to 90", numeric = true)
    EditRow("Longitude", editLon, onLon, placeholder = "-180 to 180", numeric = true)

    SectionLabel("Camera / Lens")
    EditRow("Camera Model", editCamera, onCamera)
    EditRow("Camera Make", editCameraMake, onCameraMake)
    EditRow("Lens", editLens, onLens)

    SectionLabel("Exposure")
    EditRow("ISO", editIso, onIso, numeric = true)
    EditRow("F-Number", editFNumber, onFNumber, placeholder = "e.g. 2.8", numeric = true)
    EditRow("Exposure Time", editExposureTime, onExposureTime, placeholder = "e.g. 1/250")
    EditRow("Focal Length", editFocalLength, onFocalLength, placeholder = "mm", numeric = true)
    EditRow("Exposure Bias", editExposureBias, onExposureBias, placeholder = "EV", numeric = true)
    EditRow("Exposure Prog", editExposureProgram, onExposureProgram, placeholder = "e.g. Aperture priority")
    EditRow("Metering", editMeteringMode, onMeteringMode, placeholder = "e.g. Multi-segment")
    EditRow("Flash", editFlash, onFlash, placeholder = "e.g. No Flash")
    EditRow("White Balance", editWhiteBalance, onWhiteBalance, placeholder = "e.g. Auto")

    SectionLabel("Other")
    EditRow("Orientation", editOrientation, onOrientation, placeholder = "1-8", numeric = true)
    EditRow("Color Space", editColorSpace, onColorSpace, placeholder = "e.g. sRGB")
    EditRow("Scene Type", editSceneType, onSceneType)
    EditRow("Digital Zoom", editDigitalZoom, onDigitalZoom, numeric = true)
    EditRow("Software", editSoftware, onSoftware)
    EditRow("Artist", editArtist, onArtist)
    EditRow("Copyright", editCopyright, onCopyright)

    Row(
        modifier = Modifier.fillMaxWidth().padding(top = 10.dp),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        SpButton(
            text = if (saving) "Saving..." else "Save",
            onClick = onSaveClick,
            variant = SpButtonVariant.Primary,
            enabled = !saving,
            modifier = Modifier.weight(1f),
        )
        SpButton(
            text = "Cancel",
            onClick = onCancel,
            variant = SpButtonVariant.Secondary,
            enabled = !saving,
            modifier = Modifier.weight(1f),
        )
    }
}

// ── Small building blocks ────────────────────────────────────────────────────

@Composable
private fun InfoRow(label: String, value: String) {
    Row(
        modifier = Modifier.fillMaxWidth().padding(vertical = 3.dp),
        horizontalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Text(label, color = SpViewer.textMuted, fontSize = 13.sp)
        Text(
            value,
            color = SpViewer.textPrimary,
            fontSize = 13.sp,
            textAlign = TextAlign.End,
            modifier = Modifier.weight(1f),
        )
    }
}

@Composable
private fun SectionLabel(text: String) {
    Text(
        text.uppercase(),
        color = SpViewer.textSubtle,
        fontSize = 10.sp,
        fontWeight = FontWeight.SemiBold,
        letterSpacing = 1.sp,
        modifier = Modifier.padding(top = 8.dp, bottom = 2.dp),
    )
}

@Composable
private fun SectionDivider() {
    Box(
        modifier = Modifier
            .fillMaxWidth()
            .padding(vertical = 4.dp)
            .height(1.dp)
            .background(SpViewer.divider),
    )
}

@Composable
private fun HorizontalDividerThin() {
    Box(modifier = Modifier.fillMaxWidth().height(1.dp).background(SpViewer.divider))
}

@Composable
private fun EditRow(
    label: String,
    value: String,
    onValueChange: (String) -> Unit,
    placeholder: String = "",
    numeric: Boolean = false,
) {
    Row(
        modifier = Modifier.fillMaxWidth().padding(vertical = 2.dp),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(label, color = SpViewer.textMuted, fontSize = 11.sp)
        Box(
            modifier = Modifier
                .weight(1f)
                .clip(RoundedCornerShape(6.dp))
                .background(SpViewer.inputBg)
                .border(1.dp, SpViewer.inputBorder, RoundedCornerShape(6.dp))
                .padding(horizontal = 8.dp, vertical = 6.dp),
        ) {
            BasicTextField(
                value = value,
                onValueChange = onValueChange,
                singleLine = true,
                textStyle = TextStyle(
                    color = SpViewer.textPrimary,
                    fontSize = 12.sp,
                    textAlign = TextAlign.End,
                ),
                cursorBrush = SolidColor(SpViewer.accent),
                keyboardOptions = KeyboardOptions(
                    keyboardType = if (numeric) KeyboardType.Number else KeyboardType.Text,
                ),
                modifier = Modifier.fillMaxWidth(),
                decorationBox = { inner ->
                    Box(contentAlignment = Alignment.CenterEnd) {
                        if (value.isEmpty() && placeholder.isNotEmpty()) {
                            Text(placeholder, color = SpViewer.textSubtle, fontSize = 12.sp)
                        }
                        inner()
                    }
                },
            )
        }
    }
}

@Composable
private fun DropdownRow(
    label: String,
    value: String,
    options: List<Pair<String, String>>,
    onSelect: (String) -> Unit,
) {
    var expanded by remember { mutableStateOf(false) }
    val selectedLabel = options.firstOrNull { it.first == value }?.second ?: options.first().second
    Row(
        modifier = Modifier.fillMaxWidth().padding(vertical = 2.dp),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(label, color = SpViewer.textMuted, fontSize = 11.sp)
        Box(modifier = Modifier.weight(1f)) {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .clip(RoundedCornerShape(6.dp))
                    .background(SpViewer.inputBg)
                    .border(1.dp, SpViewer.inputBorder, RoundedCornerShape(6.dp))
                    .clickable { expanded = true }
                    .padding(start = 8.dp, end = 4.dp, top = 6.dp, bottom = 6.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    selectedLabel,
                    color = SpViewer.textPrimary,
                    fontSize = 12.sp,
                    textAlign = TextAlign.End,
                    modifier = Modifier.weight(1f),
                )
                Icon(
                    Icons.Default.ArrowDropDown,
                    contentDescription = null,
                    tint = SpViewer.textMuted,
                    modifier = Modifier.size(18.dp),
                )
            }
            DropdownMenu(expanded = expanded, onDismissRequest = { expanded = false }) {
                options.forEach { (optValue, optLabel) ->
                    DropdownMenuItem(
                        text = { Text(optLabel) },
                        onClick = { onSelect(optValue); expanded = false },
                    )
                }
            }
        }
    }
}
