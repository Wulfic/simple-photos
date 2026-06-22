/**
 * Sphere360View — interactive WebGL-style 360° photo-sphere for Android.
 *
 * Mirrors the web's [Sphere360Viewer]: an equirectangular image is textured
 * onto the inside of a UV sphere with the camera at the centre.  Drag looks
 * around (yaw / pitch), pinch zooms the field of view.  This replaces the old
 * flat horizontal-pan fallback that the Android viewer used for 360s — that
 * never gave the look-around experience the web has.
 *
 * Rendering is OpenGL ES 2.0 on a [GLSurfaceView], wrapped in an AndroidView
 * exactly like [MotionPhotoOverlay] wraps a Media3 PlayerView — that surface /
 * Compose-overlay layering is already proven in this app, so the "Full View"
 * pill drawn on top stays visible and tappable.
 *
 * Texture is decoded through Coil (so AVIF / HEIF panoramas decode) at a capped
 * [MAX_PANO_DECODE_PX] longest edge so the GPU upload stays within the
 * GL_MAX_TEXTURE_SIZE / memory budget — same cap the flat pano path uses.
 */
package com.simplephotos.ui.screens.viewer

import android.content.Context
import android.graphics.Bitmap
import android.opengl.GLES20
import android.opengl.GLSurfaceView
import android.opengl.GLUtils
import android.opengl.Matrix
import android.util.Log
import android.view.MotionEvent
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import coil.imageLoader
import coil.request.ImageRequest
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.FloatBuffer
import java.nio.ShortBuffer
import javax.microedition.khronos.egl.EGLConfig
import javax.microedition.khronos.opengles.GL10
import kotlin.math.cos
import kotlin.math.hypot
import kotlin.math.sin
import kotlin.math.PI

private const val TAG_SPHERE = "Sphere360View"

/**
 * Describe a decoded image blob by its magic bytes + size, for diagnosing
 * decode failures (e.g. a 360 panorama rendering black). Identifies the
 * container so we can tell "AVIF the device can't software-decode" apart from
 * "truncated/empty blob". Used by the secure viewer + the 360 sphere.
 */
internal fun describeImageBytes(b: ByteArray): String {
    fun ascii(off: Int, len: Int): String =
        if (b.size >= off + len) String(b, off, len, Charsets.US_ASCII) else ""
    val fmt = when {
        b.size < 12 -> "too-small(${b.size}B)"
        b[0] == 0xFF.toByte() && b[1] == 0xD8.toByte() -> "JPEG"
        b[0] == 0x89.toByte() && b[1] == 0x50.toByte() -> "PNG"
        ascii(0, 3) == "GIF" -> "GIF"
        ascii(0, 4) == "RIFF" && ascii(8, 4) == "WEBP" -> "WEBP"
        ascii(4, 4) == "ftyp" -> "ISO-BMFF/${ascii(8, 4)}" // avif/heic/mif1…
        b[0] == 0x42.toByte() && b[1] == 0x4D.toByte() -> "BMP"
        else -> "unknown(${"%02x%02x%02x%02x".format(b[0], b[1], b[2], b[3])})"
    }
    return "$fmt, ${b.size} bytes"
}

// ─── Composable wrapper ──────────────────────────────────────────────────────

/**
 * Live 360° sphere overlay.  Loads [imageData] (a content [android.net.Uri] for
 * local files or a decrypted [ByteArray] for encrypted blobs), textures it onto
 * a sphere, and lets the user look around.  Tapping the pill calls
 * [onExitToFull] to drop back to the flat "Full View".
 */
@Composable
fun Sphere360Overlay(
    imageData: Any?,
    contentDescription: String,
    onExitToFull: () -> Unit,
) {
    val context = LocalContext.current
    val glView = remember { PanoSphereGLSurfaceView(context) }
    var loading by remember(imageData) { mutableStateOf(true) }
    var failed by remember(imageData) { mutableStateOf(false) }

    // Decode the panorama through Coil (handles AVIF/HEIF), capped, as a
    // software bitmap that can be uploaded to a GL texture.
    LaunchedEffect(imageData) {
        if (imageData == null) { loading = false; failed = true; return@LaunchedEffect }
        loading = true
        failed = false
        // Diagnostic: identify the source so a black 360 can be traced to a
        // format/size the device's Coil software decoder rejects (see the
        // secure-gallery 360-black bug — pano decodes, equirectangular didn't).
        if (imageData is ByteArray) {
            Log.d(TAG_SPHERE, "decoding 360: ${describeImageBytes(imageData)}")
        }
        val bmp = try {
            withContext(Dispatchers.IO) {
                val req = ImageRequest.Builder(context)
                    .data(imageData)
                    .size(MAX_PANO_DECODE_PX)
                    .allowHardware(false) // need a non-hardware Bitmap for GLUtils
                    // We recycle this bitmap after GL upload, so it must not be a
                    // shared/cached instance (would corrupt the cache + flat base).
                    .memoryCachePolicy(coil.request.CachePolicy.DISABLED)
                    .build()
                val result = context.imageLoader.execute(req)
                if (result is coil.request.ErrorResult) {
                    Log.w(TAG_SPHERE, "Coil 360 decode error: ${result.throwable.message}", result.throwable)
                }
                (result.drawable as? android.graphics.drawable.BitmapDrawable)?.bitmap
            }
        } catch (e: Throwable) {
            Log.w(TAG_SPHERE, "panorama decode failed: ${e.message}", e)
            null
        }
        if (bmp != null) {
            Log.d(TAG_SPHERE, "360 decoded to ${bmp.width}x${bmp.height}")
            glView.setPanorama(bmp)
            loading = false
        } else {
            Log.w(TAG_SPHERE, "360 decode produced no bitmap → showing failure state")
            loading = false
            failed = true
        }
    }

    // Drive the GL render thread with the composition lifecycle.
    DisposableEffect(glView) {
        glView.onResume()
        onDispose { glView.onPause() }
    }

    Box(modifier = Modifier.fillMaxSize().background(Color.Black)) {
        AndroidView(
            factory = { glView },
            modifier = Modifier.fillMaxSize()
        )

        if (loading) {
            CircularProgressIndicator(
                strokeWidth = 2.dp,
                color = Color.White,
                modifier = Modifier.align(Alignment.Center)
            )
        }

        // Visible failure state — previously a decode failure left the surface
        // pure black with no indication (the secure 360-black bug). Surface it
        // so the user (and logcat) knows the texture didn't load.
        if (failed) {
            Text(
                "Couldn't load 360° view",
                color = Color.White,
                fontSize = 13.sp,
                modifier = Modifier.align(Alignment.Center)
            )
        }

        // "360° Full View" pill — drops back to the flat fit view.
        Surface(
            modifier = Modifier
                .align(Alignment.BottomCenter)
                .padding(bottom = 80.dp)
                .clip(CircleShape)
                .clickable(onClick = onExitToFull),
            color = Color.Black.copy(alpha = 0.6f),
            shape = CircleShape
        ) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                modifier = Modifier.padding(horizontal = 14.dp, vertical = 6.dp)
            ) {
                Text("360°", color = Color.White, fontWeight = FontWeight.Bold, fontSize = 11.sp)
                Spacer(Modifier.width(8.dp))
                Text("Full View", color = Color.White, fontSize = 12.sp)
            }
        }
    }
}

// ─── GLSurfaceView + renderer ────────────────────────────────────────────────

/**
 * A [GLSurfaceView] that renders an equirectangular panorama onto the inside of
 * a sphere and converts touch gestures into camera yaw / pitch / zoom.
 */
class PanoSphereGLSurfaceView(context: Context) : GLSurfaceView(context) {
    private val renderer: SphereRenderer

    init {
        setEGLContextClientVersion(2)
        renderer = SphereRenderer { requestRender() }
        setRenderer(renderer)
        renderMode = RENDERMODE_WHEN_DIRTY
    }

    fun setPanorama(bitmap: Bitmap) {
        renderer.setPendingBitmap(bitmap)
        requestRender()
    }

    // ── Touch → look-around ──────────────────────────────────────────────────
    private var lastX = 0f
    private var lastY = 0f
    private var lastPinchDist = 0f
    private var pointerMode = MODE_NONE

    override fun onTouchEvent(event: MotionEvent): Boolean {
        when (event.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                lastX = event.x
                lastY = event.y
                pointerMode = MODE_DRAG
            }
            MotionEvent.ACTION_POINTER_DOWN -> {
                lastPinchDist = pinchDistance(event)
                pointerMode = MODE_PINCH
            }
            MotionEvent.ACTION_MOVE -> {
                if (pointerMode == MODE_PINCH && event.pointerCount >= 2) {
                    val dist = pinchDistance(event)
                    if (lastPinchDist > 0f && dist > 0f) {
                        // Pinch out (dist grows) → smaller FOV (zoom in).
                        renderer.zoomBy(lastPinchDist / dist)
                        requestRender()
                    }
                    lastPinchDist = dist
                } else if (pointerMode == MODE_DRAG) {
                    val dx = event.x - lastX
                    val dy = event.y - lastY
                    lastX = event.x
                    lastY = event.y
                    renderer.rotateBy(dx, dy)
                    requestRender()
                }
            }
            MotionEvent.ACTION_POINTER_UP -> {
                // Fall back to single-finger drag using the remaining pointer.
                val remaining = if (event.actionIndex == 0) 1 else 0
                lastX = event.getX(remaining)
                lastY = event.getY(remaining)
                pointerMode = MODE_DRAG
            }
            MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL -> {
                pointerMode = MODE_NONE
            }
        }
        return true
    }

    private fun pinchDistance(event: MotionEvent): Float {
        if (event.pointerCount < 2) return 0f
        return hypot(event.getX(0) - event.getX(1), event.getY(0) - event.getY(1))
    }

    private companion object {
        const val MODE_NONE = 0
        const val MODE_DRAG = 1
        const val MODE_PINCH = 2
    }
}

/**
 * Renders the textured sphere from the inside.  Camera sits at the origin; yaw
 * and pitch rotate the world, FOV controls zoom.  Thread-safety: gesture state
 * (yaw/pitch/fov) and the pending bitmap are read/written across the UI and GL
 * threads, so they are marked @Volatile and the bitmap upload is done on the GL
 * thread inside onDrawFrame.
 */
private class SphereRenderer(
    private val onNeedsRender: () -> Unit,
) : GLSurfaceView.Renderer {

    @Volatile private var pendingBitmap: Bitmap? = null
    @Volatile private var yaw = 0f
    @Volatile private var pitch = 0f
    @Volatile private var fov = 75f

    private var program = 0
    private var aPosLoc = 0
    private var aUvLoc = 0
    private var uMvpLoc = 0
    private var uTexLoc = 0
    private var textureId = 0
    private var hasTexture = false
    // GL_MAX_TEXTURE_SIZE for this GPU, read after the context exists. A texture
    // larger than this uploads as BLACK with no error — a prime suspect for the
    // equirectangular-renders-black bug, since 360s are wider than flat panos.
    private var maxTextureSize = 2048

    private lateinit var vertexBuffer: FloatBuffer
    private lateinit var indexBuffer: ShortBuffer
    private var indexCount = 0

    private val projMatrix = FloatArray(16)
    private val viewMatrix = FloatArray(16)
    private val mvpMatrix = FloatArray(16)
    private var aspect = 1f

    fun setPendingBitmap(bitmap: Bitmap) { pendingBitmap = bitmap }

    fun rotateBy(dx: Float, dy: Float) {
        // Drag right → look right.  Scale by FOV so the feel is constant across
        // zoom levels (a small FOV moves slower per pixel).
        val factor = fov / 900f
        yaw += dx * factor * -1f
        pitch = (pitch + dy * factor).coerceIn(-89f, 89f)
    }

    fun zoomBy(scale: Float) {
        fov = (fov * scale).coerceIn(25f, 100f)
    }

    override fun onSurfaceCreated(gl: GL10?, config: EGLConfig?) {
        GLES20.glClearColor(0f, 0f, 0f, 1f)
        GLES20.glEnable(GLES20.GL_DEPTH_TEST)
        // We view the sphere from the inside, so the outward-facing triangles
        // are seen from behind — just draw both sides rather than flip winding.
        GLES20.glDisable(GLES20.GL_CULL_FACE)

        program = buildProgram(VERTEX_SHADER, FRAGMENT_SHADER)
        aPosLoc = GLES20.glGetAttribLocation(program, "aPos")
        aUvLoc = GLES20.glGetAttribLocation(program, "aUv")
        uMvpLoc = GLES20.glGetUniformLocation(program, "uMvp")
        uTexLoc = GLES20.glGetUniformLocation(program, "uTex")

        buildSphere(STACKS, SLICES)

        val tex = IntArray(1)
        GLES20.glGenTextures(1, tex, 0)
        textureId = tex[0]
        hasTexture = false

        val maxTex = IntArray(1)
        GLES20.glGetIntegerv(GLES20.GL_MAX_TEXTURE_SIZE, maxTex, 0)
        if (maxTex[0] > 0) maxTextureSize = maxTex[0]
        Log.d(TAG_SPHERE, "GL_MAX_TEXTURE_SIZE = $maxTextureSize")
    }

    override fun onSurfaceChanged(gl: GL10?, width: Int, height: Int) {
        GLES20.glViewport(0, 0, width, height)
        aspect = if (height > 0) width.toFloat() / height.toFloat() else 1f
    }

    override fun onDrawFrame(gl: GL10?) {
        // Upload a freshly-decoded bitmap on the GL thread, if one is pending.
        pendingBitmap?.let { rawBmp ->
            pendingBitmap = null
            // A texture wider/taller than GL_MAX_TEXTURE_SIZE silently uploads as
            // black. Coil caps to MAX_PANO_DECODE_PX (4096), but a GPU may report
            // a smaller max — downscale to fit so the sphere never goes black.
            val bmp = if (rawBmp.width > maxTextureSize || rawBmp.height > maxTextureSize) {
                val scale = maxTextureSize.toFloat() / maxOf(rawBmp.width, rawBmp.height)
                val w = (rawBmp.width * scale).toInt().coerceAtLeast(1)
                val h = (rawBmp.height * scale).toInt().coerceAtLeast(1)
                Log.w(TAG_SPHERE, "360 texture ${rawBmp.width}x${rawBmp.height} exceeds maxTex=$maxTextureSize → scaling to ${w}x$h")
                Bitmap.createScaledBitmap(rawBmp, w, h, true).also {
                    if (it != rawBmp && !rawBmp.isRecycled) rawBmp.recycle()
                }
            } else rawBmp
            GLES20.glBindTexture(GLES20.GL_TEXTURE_2D, textureId)
            GLES20.glTexParameteri(GLES20.GL_TEXTURE_2D, GLES20.GL_TEXTURE_MIN_FILTER, GLES20.GL_LINEAR)
            GLES20.glTexParameteri(GLES20.GL_TEXTURE_2D, GLES20.GL_TEXTURE_MAG_FILTER, GLES20.GL_LINEAR)
            // CLAMP_TO_EDGE is NPOT-safe (panoramas aren't guaranteed power-of-two).
            GLES20.glTexParameteri(GLES20.GL_TEXTURE_2D, GLES20.GL_TEXTURE_WRAP_S, GLES20.GL_CLAMP_TO_EDGE)
            GLES20.glTexParameteri(GLES20.GL_TEXTURE_2D, GLES20.GL_TEXTURE_WRAP_T, GLES20.GL_CLAMP_TO_EDGE)
            GLUtils.texImage2D(GLES20.GL_TEXTURE_2D, 0, bmp, 0)
            val err = GLES20.glGetError()
            if (err != GLES20.GL_NO_ERROR) Log.w(TAG_SPHERE, "texImage2D GL error: 0x${err.toString(16)}")
            hasTexture = true
            if (!bmp.isRecycled) bmp.recycle()
        }

        GLES20.glClear(GLES20.GL_COLOR_BUFFER_BIT or GLES20.GL_DEPTH_BUFFER_BIT)
        if (!hasTexture) return

        // Projection from current FOV (vertical).
        Matrix.perspectiveM(projMatrix, 0, fov, aspect, 0.1f, 100f)

        // View = rotate world by pitch then yaw (camera at origin looking -Z).
        Matrix.setIdentityM(viewMatrix, 0)
        Matrix.rotateM(viewMatrix, 0, pitch, 1f, 0f, 0f)
        Matrix.rotateM(viewMatrix, 0, yaw, 0f, 1f, 0f)
        Matrix.multiplyMM(mvpMatrix, 0, projMatrix, 0, viewMatrix, 0)

        GLES20.glUseProgram(program)
        GLES20.glUniformMatrix4fv(uMvpLoc, 1, false, mvpMatrix, 0)

        GLES20.glActiveTexture(GLES20.GL_TEXTURE0)
        GLES20.glBindTexture(GLES20.GL_TEXTURE_2D, textureId)
        GLES20.glUniform1i(uTexLoc, 0)

        vertexBuffer.position(0)
        GLES20.glEnableVertexAttribArray(aPosLoc)
        GLES20.glVertexAttribPointer(aPosLoc, 3, GLES20.GL_FLOAT, false, STRIDE, vertexBuffer)

        vertexBuffer.position(3)
        GLES20.glEnableVertexAttribArray(aUvLoc)
        GLES20.glVertexAttribPointer(aUvLoc, 2, GLES20.GL_FLOAT, false, STRIDE, vertexBuffer)

        GLES20.glDrawElements(GLES20.GL_TRIANGLES, indexCount, GLES20.GL_UNSIGNED_SHORT, indexBuffer)

        GLES20.glDisableVertexAttribArray(aPosLoc)
        GLES20.glDisableVertexAttribArray(aUvLoc)
    }

    // ── Geometry ──────────────────────────────────────────────────────────────
    private fun buildSphere(stacks: Int, slices: Int) {
        val verts = ArrayList<Float>((stacks + 1) * (slices + 1) * 5)
        for (i in 0..stacks) {
            val phi = PI * i / stacks            // 0 (top) .. PI (bottom)
            val sinPhi = sin(phi)
            val cosPhi = cos(phi)
            for (j in 0..slices) {
                val theta = 2.0 * PI * j / slices
                val x = (sinPhi * cos(theta)).toFloat()
                val y = cosPhi.toFloat()
                val z = (sinPhi * sin(theta)).toFloat()
                // Flip U because we sample the texture from the inside of the
                // sphere; otherwise text reads mirrored.  V: image top → phi 0.
                val u = 1f - j.toFloat() / slices
                val v = i.toFloat() / stacks
                verts.add(x); verts.add(y); verts.add(z); verts.add(u); verts.add(v)
            }
        }
        val idx = ArrayList<Short>(stacks * slices * 6)
        val cols = slices + 1
        for (i in 0 until stacks) {
            for (j in 0 until slices) {
                val a = (i * cols + j)
                val b = a + cols
                idx.add(a.toShort()); idx.add(b.toShort()); idx.add((a + 1).toShort())
                idx.add((a + 1).toShort()); idx.add(b.toShort()); idx.add((b + 1).toShort())
            }
        }
        indexCount = idx.size

        val vb = ByteBuffer.allocateDirect(verts.size * 4).order(ByteOrder.nativeOrder()).asFloatBuffer()
        for (f in verts) vb.put(f)
        vb.position(0)
        vertexBuffer = vb

        val ib = ByteBuffer.allocateDirect(idx.size * 2).order(ByteOrder.nativeOrder()).asShortBuffer()
        for (s in idx) ib.put(s)
        ib.position(0)
        indexBuffer = ib
    }

    // ── Shader helpers ──────────────────────────────────────────────────────
    private fun buildProgram(vsSource: String, fsSource: String): Int {
        val vs = compileShader(GLES20.GL_VERTEX_SHADER, vsSource)
        val fs = compileShader(GLES20.GL_FRAGMENT_SHADER, fsSource)
        val prog = GLES20.glCreateProgram()
        GLES20.glAttachShader(prog, vs)
        GLES20.glAttachShader(prog, fs)
        GLES20.glLinkProgram(prog)
        val status = IntArray(1)
        GLES20.glGetProgramiv(prog, GLES20.GL_LINK_STATUS, status, 0)
        if (status[0] == 0) {
            Log.e(TAG_SPHERE, "program link failed: ${GLES20.glGetProgramInfoLog(prog)}")
            GLES20.glDeleteProgram(prog)
            return 0
        }
        return prog
    }

    private fun compileShader(type: Int, source: String): Int {
        val shader = GLES20.glCreateShader(type)
        GLES20.glShaderSource(shader, source)
        GLES20.glCompileShader(shader)
        val status = IntArray(1)
        GLES20.glGetShaderiv(shader, GLES20.GL_COMPILE_STATUS, status, 0)
        if (status[0] == 0) {
            Log.e(TAG_SPHERE, "shader compile failed: ${GLES20.glGetShaderInfoLog(shader)}")
            GLES20.glDeleteShader(shader)
            return 0
        }
        return shader
    }

    private companion object {
        const val STACKS = 48
        const val SLICES = 96
        const val STRIDE = 5 * 4 // 3 pos + 2 uv floats

        const val VERTEX_SHADER = """
            uniform mat4 uMvp;
            attribute vec4 aPos;
            attribute vec2 aUv;
            varying vec2 vUv;
            void main() {
                vUv = aUv;
                gl_Position = uMvp * aPos;
            }
        """

        const val FRAGMENT_SHADER = """
            precision mediump float;
            uniform sampler2D uTex;
            varying vec2 vUv;
            void main() {
                gl_FragColor = texture2D(uTex, vUv);
            }
        """
    }
}
