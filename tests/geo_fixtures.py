"""Single source of truth for the geolocation **edge-case** sample set.

Consumed by:
  * ``tests/test_86_geo_albums_edgecases.py`` — E2E regression coverage that
    drives the real offline geocoder and asserts every boundary below.
  * ``scripts/generate_geo_samples.py`` — writes captioned, geotagged sample
    photos to ``~/Desktop/Sample_files/geo`` for manual UI testing.

Both import the SAME groups and the SAME byte generator so what you eyeball in
the desktop app is exactly what the test asserts.

The server-side clustering rules these groups exercise (see
``server/src/geo/handlers.rs``):

    Trip   = >= 5 photos, same city, consecutive photos <= 3 days apart
             (MAX_GAP_DAYS = 3, MIN_PHOTOS = 5)
    Memory = >= 3 photos, same city, same calendar day (cnt >= 3)

Every group below pins one boundary of those rules, or a negative path
(no GPS, no date, unresolvable coordinates, below threshold).  Photos inside a
group share IDENTICAL coordinates, so they all resolve to the same city and the
expected counts are independent of exactly which city name the dataset picks.
"""

from __future__ import annotations

import io
from dataclasses import dataclass
from typing import List, Optional, Tuple

from helpers import build_datetime_exif_app1, build_gps_exif_app1


@dataclass(frozen=True)
class GeoGroup:
    key: str                              # short id; used in sample filenames
    label: str                            # city caption drawn on the photo
    lat: Optional[float]                  # None  => photo carries no GPS
    lon: Optional[float]
    dates: Tuple[Optional[str], ...]      # one EXIF date per photo; None => undated
    edge_case: str                        # the rule/boundary this proves
    expect: str                           # human-readable expected outcome
    precise_label: Optional[str] = None   # street address once precise is on

    @property
    def has_gps(self) -> bool:
        return self.lat is not None and self.lon is not None

    @property
    def count(self) -> int:
        return len(self.dates)


# ── The itinerary ────────────────────────────────────────────────────────
# Coordinates are dead-centre on well-known landmarks so cities500 resolves
# them cleanly; the precise group's coordinate returns a real street address
# from Nominatim/Photon ("350 5th Avenue").

GROUPS: List[GeoGroup] = [
    GeoGroup(
        key="paris_trip", label="Paris, France",
        lat=48.8584, lon=2.2945,                     # Eiffel Tower
        dates=(
            "2025:06:06 09:00:00", "2025:06:06 13:00:00", "2025:06:06 19:00:00",
            "2025:06:07 10:00:00", "2025:06:07 15:00:00", "2025:06:07 20:00:00",
        ),
        edge_case="Multi-day trip + per-day memories",
        expect="1 trip (6 photos, 2 days) + 2 memories (3 + 3)",
    ),
    GeoGroup(
        key="tokyo_burst", label="Tokyo, Japan",
        lat=35.6595, lon=139.7006,                   # Shibuya
        dates=(
            "2025:09:04 08:00:00", "2025:09:04 11:00:00", "2025:09:04 13:00:00",
            "2025:09:04 17:00:00", "2025:09:04 20:00:00",
        ),
        edge_case="Single-day burst still qualifies as a trip",
        expect="1 trip (5 photos, 1 day) + 1 memory (5)",
    ),
    GeoGroup(
        key="rome_memory", label="Rome, Italy",
        lat=41.8902, lon=12.4922,                    # Colosseum
        dates=("2025:07:12 10:00:00", "2025:07:12 14:00:00", "2025:07:12 19:00:00"),
        edge_case="Exactly 3 photos/day = memory, but < 5 = NO trip",
        expect="1 memory (3), NO trip",
    ),
    GeoGroup(
        key="barcelona_gap", label="Barcelona, Spain",
        lat=41.3851, lon=2.1734,                     # Plaça de Catalunya
        dates=(
            "2025:08:01 09:00:00", "2025:08:01 13:00:00", "2025:08:01 18:00:00",
            "2025:08:04 10:00:00", "2025:08:04 16:00:00",   # exactly 3-day gap
        ),
        edge_case="Gap of exactly MAX_GAP_DAYS (3) stays ONE trip",
        expect="1 trip (5 photos, spans 4 days) + 1 memory (Aug 1 = 3); Aug 4 (2) makes no memory",
    ),
    GeoGroup(
        key="amsterdam_twice", label="Amsterdam, Netherlands",
        lat=52.3676, lon=4.9041,                     # Museumplein
        dates=(
            "2025:10:01 09:00:00", "2025:10:01 13:00:00", "2025:10:01 18:00:00",
            "2025:10:02 10:00:00", "2025:10:02 15:00:00",
            "2025:10:15 09:00:00", "2025:10:15 13:00:00", "2025:10:15 18:00:00",
            "2025:10:16 10:00:00", "2025:10:16 15:00:00",
        ),
        edge_case="Same city, two visits >3 days apart = TWO trips",
        expect="2 trips (5 + 5) + 2 memories (Oct 1 = 3, Oct 15 = 3)",
    ),
    GeoGroup(
        key="lisbon_below", label="Lisbon, Portugal",
        lat=38.7223, lon=-9.1393,                    # Praça do Comércio
        dates=("2025:05:20 11:00:00", "2025:05:20 16:00:00"),
        edge_case="Below memory threshold (2 < 3)",
        expect="Resolves a location, but NO memory and NO trip",
    ),
    GeoGroup(
        key="sydney_nodate", label="Sydney, Australia",
        lat=-33.8568, lon=151.2153,                  # Opera House
        dates=(None, None, None, None, None),        # GPS but no DateTimeOriginal
        edge_case="GPS but no EXIF date -> server stamps taken_at = import time",
        expect="Dates to the import day, so these 5 cluster into 1 trip + 1 memory "
               "labelled with whenever you imported them (NOT their real date)",
    ),
    GeoGroup(
        key="timeline_nogps", label="No GPS (camera scan)",
        lat=None, lon=None,                          # no GPS at all
        dates=(
            "2025:03:10 09:00:00", "2025:03:11 09:00:00", "2025:03:12 09:00:00",
            "2025:03:13 09:00:00", "2025:03:14 09:00:00",
        ),
        edge_case="Has a date but no location",
        expect="Appears in the timeline only - never a location album",
    ),
    GeoGroup(
        key="ocean_sentinel", label="Mid-Pacific (no city)",
        lat=0.0, lon=-140.0,                         # open ocean, no city nearby
        dates=("2025:04:02 10:00:00", "2025:04:02 12:00:00", "2025:04:02 14:00:00"),
        edge_case="Coordinates resolve to NO city (geo_city='' sentinel)",
        expect="Stored with coords but never an album, and never sticks the resolver",
    ),
    GeoGroup(
        key="nyc_precise", label="New York City, USA",
        lat=40.7484, lon=-73.9857,                   # Empire State Building
        dates=("2025:11:21 10:00:00", "2025:11:21 14:00:00", "2025:11:21 21:00:00"),
        edge_case="Precise (street-level) reverse geocoding",
        expect="1 memory (3); with Precise ON the title becomes the street address",
        precise_label="350 5th Avenue, New York",
    ),
]


# ── Aggregate expectations once fully resolved ───────────────────────────
# Counts are city-name-independent (see module docstring).  We split albums
# into two buckets so the assertions are fully deterministic:
#
#   * "dated" albums come from groups with explicit EXIF dates (all in 2025).
#     Their start day falls in EXPLICIT_DAYS — fixed, reproducible counts.
#   * the sydney_nodate group has no EXIF date, so the server stamps it with
#     the import time; its album lands on a day NOT in EXPLICIT_DAYS.  We
#     assert it separately (exactly one extra 5-photo trip + 5-photo memory)
#     instead of pinning it to a wall-clock day.
EXPECT_TRIP_PHOTO_COUNTS_DATED: List[int] = [5, 5, 5, 5, 6]
EXPECT_MEMORY_PHOTO_COUNTS_DATED: List[int] = [3, 3, 3, 3, 3, 3, 3, 5]
EXPECT_IMPORT_DATED_TRIP_COUNT: int = 5      # sydney_nodate: 5 photos, 1 day
EXPECT_IMPORT_DATED_MEMORY_COUNT: int = 5
EXPECT_LOCATION_CITY_COUNT: int = 8          # 8 groups carry resolvable GPS coords

# Every explicit calendar day we upload a photo for (used to separate the
# import-dated no-date group from the deterministic dated albums).
EXPLICIT_DAYS = frozenset(
    d[:10].replace(":", "-") for g in GROUPS for d in g.dates if d is not None
)


def total_photos() -> int:
    return sum(g.count for g in GROUPS)


def memory_day(memory: dict) -> str:
    """The calendar day (YYYY-MM-DD) a /api/geo/memories entry belongs to.

    The server encodes it as the suffix of the memory id (``<city>_<day>``);
    the city slug never contains an underscore, so the last segment is the day.
    """
    return memory["id"].rsplit("_", 1)[-1]


def photo_bytes(group: GeoGroup, idx: int, seq: int, *, captioned: bool) -> bytes:
    """Render one photo for ``group`` and splice in the right EXIF.

    ``idx``  — index within the group (selects the date).
    ``seq``  — globally-unique sequence number; folded into the pixels so no
               two photos are byte-identical (the server content-hash-dedups
               identical uploads, which would otherwise collapse the set).
    ``captioned`` — True for the human-facing desktop samples (big labelled
               image); False for the tiny images the E2E test uploads.
    """
    from PIL import Image

    date_str = group.dates[idx]

    if captioned:
        img = _render_caption(group, idx, seq)
    else:
        # Tiny solid image; size varies with seq so the bytes hash uniquely.
        img = Image.new("RGB", (8 + seq, 8 + seq), (64, 128, 32))

    buf = io.BytesIO()
    img.save(buf, format="JPEG", quality=90)
    jpeg = buf.getvalue()

    if group.has_gps:
        app1 = build_gps_exif_app1(group.lat, group.lon, date_str)
    elif date_str is not None:
        app1 = build_datetime_exif_app1(date_str)
    else:  # pragma: no cover — no group is both GPS-less and date-less
        return jpeg

    return jpeg[:2] + app1 + jpeg[2:]


def _render_caption(group: GeoGroup, idx: int, seq: int):
    """Draw a human-readable card so each sample explains its own edge case."""
    from PIL import Image, ImageDraw

    w, h = 1024, 768
    # Deterministic per-group background tint for visual grouping.
    base = (sum(ord(c) for c in group.key) * 37) % 360
    bg = _hsl_to_rgb(base, 0.30, 0.22)
    img = Image.new("RGB", (w, h), bg)
    d = ImageDraw.Draw(img)

    date_str = group.dates[idx] or "(no date / no EXIF timestamp)"
    coord = (f"{group.lat:.4f}, {group.lon:.4f}" if group.has_gps
             else "(no GPS)")

    lines = [
        (group.label, 3),
        (f"{date_str}", 1),
        (f"GPS: {coord}", 1),
        ("", 1),
        (f"edge case: {group.edge_case}", 1),
        (f"expect: {group.expect}", 1),
    ]
    if group.precise_label:
        lines.append((f"precise: {group.precise_label}", 1))
    lines.append((f"[{group.key}  photo {idx + 1}/{group.count}  #{seq}]", 1))

    y = 60
    for text, scale in lines:
        if not text:
            y += 24
            continue
        _draw_scaled(img, d, (60, y), text, scale, fill=(235, 235, 240))
        y += 28 * scale + 14

    # A few seq-derived pixels guarantee byte-level uniqueness even if two
    # captions ever matched.
    for k in range(8):
        img.putpixel((k, 0), ((seq * 71 + k) % 256, (seq * 13) % 256, k * 9 % 256))
    return img


def _draw_scaled(img, draw, xy, text, scale, fill):
    """Draw text at an integer scale using PIL's bundled bitmap font (no
    external font files needed, so this works on any machine)."""
    from PIL import Image, ImageDraw

    if scale <= 1:
        draw.text(xy, text, fill=fill)
        return
    tmp = Image.new("RGBA", (len(text) * 7 + 4, 12), (0, 0, 0, 0))
    ImageDraw.Draw(tmp).text((0, 0), text, fill=fill)
    tmp = tmp.resize((tmp.width * scale, tmp.height * scale), Image.NEAREST)
    img.paste(tmp, (int(xy[0]), int(xy[1])), tmp)


def _hsl_to_rgb(h: float, s: float, light: float):
    import colorsys
    r, g, b = colorsys.hls_to_rgb(h / 360.0, light, s)
    return (int(r * 255), int(g * 255), int(b * 255))
