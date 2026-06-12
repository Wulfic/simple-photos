---
name: ddt-test-pattern
description: "Use when the user needs to write parametrized / data-driven tests for simple-photos. Examples: \"Cover all edge cases for X\", \"Add boundary value tests\", \"Parametrize this test\", \"Write DDT for this endpoint\", \"Add test rows for invalid inputs\""
---

# DDT (Data-Driven Test) Pattern

## When to Use

Every code path that accepts variable input must be covered by a parametrize table. Use this pattern when:
- Testing the same logic with different inputs (boundary values, valid/invalid, min/max)
- Multiple similar tests can share the same `def test_...` body
- You are extending an existing DDT file with more cases

## The Pattern (follow exactly)

```python
import pytest
from helpers import APIClient, generate_test_jpeg

# ══════════════════════════════════════════════════════════════════════
# DDT: <Group name — one section per logical group>
# ══════════════════════════════════════════════════════════════════════

BRIGHTNESS_CASES = [                           # SCREAMING_SNAKE_CASE name
    pytest.param(1,    id="brightness_+1_min_nondefault"),   # descriptive id
    pytest.param(-1,   id="brightness_-1_min_nondefault"),
    pytest.param(100,  id="brightness_+100_max"),
    pytest.param(-100, id="brightness_-100_min"),
    pytest.param(0,    id="brightness_0_default"),
]


class TestBrightnessSetCrop:
    """<One-line description of what this class tests>"""

    @pytest.mark.parametrize("brightness", BRIGHTNESS_CASES)  # param names match table
    def test_brightness_round_trip(self, user_client, brightness):
        # Arrange
        pid = _upload(user_client)
        meta = {"x": 0.0, "y": 0.0, "width": 1.0, "height": 1.0,
                "rotate": 0, "brightness": brightness}
        # Act
        result = user_client.crop_photo(pid, json.dumps(meta))
        # Assert
        assert result["id"] == pid
        persisted = _get_crop(user_client, pid)
        assert persisted is not None
        assert persisted["brightness"] == brightness
```

## Rules

1. **One constant table per logical group** — `BRIGHTNESS_CASES`, `ROTATION_CASES`, etc.
2. **Every `pytest.param(...)` must have `id=`** — descriptive, slug-style (`what_value_scenario`)
3. **Shared test body** — only inputs differ per row; the logic never branches per test
4. **Boundary values are mandatory rows**: min valid, max valid, min invalid, max invalid, zero/default
5. **Class name describes the group** — `TestBrightnessSetCrop`, `TestRotationDimensions`
6. **Prefer extending existing tables** over new one-off test functions

## Multi-parameter tables

For multiple parameters, use tuples and unpack in `@pytest.mark.parametrize`:

```python
CROP_DIM_CASES = [
    #  src_w, src_h, rotate, exp_w, exp_h
    pytest.param(100, 80, 0,   100, 80,  id="100x80_rot0"),
    pytest.param(100, 80, 90,  80,  100, id="100x80_rot90"),
    pytest.param(200, 200, 90, 200, 200, id="square_rot90"),
]

class TestCropDimensions:
    @pytest.mark.parametrize("src_w,src_h,rotate,exp_w,exp_h", CROP_DIM_CASES)
    def test_output_dimensions(self, user_client, src_w, src_h, rotate, exp_w, exp_h):
        pid = _upload(user_client, src_w, src_h)
        w, h = _dup_dims(user_client, pid, {"rotate": rotate, ...})
        assert w == exp_w
        assert h == exp_h
```

## id= naming conventions

| Pattern | Example |
|---------|---------|
| `<field>_<value>_<label>` | `brightness_+100_max` |
| `<w>x<h>_rot<deg>` | `100x80_rot90` |
| `<type>_<variant>` | `mime_png_valid` |
| `<status>_<label>` | `http_400_missing_field` |

## Checklist

```
- [ ] One named constant table per logical group
- [ ] Every pytest.param has a descriptive id=
- [ ] Test class name matches the group
- [ ] Boundary + invalid + default values each have a row
- [ ] Test body is shared logic — no per-row branching
- [ ] Ran pytest tests/test_NN_*.py -v --tb=short, all rows pass
- [ ] Checked existing DDT files (test_35, test_38, test_40–44) for similar tables before creating new ones
```

## Reference files

| File | Covers |
|------|--------|
| `tests/test_35_edit_save_ddt.py` | Crop metadata, brightness, rotation |
| `tests/test_38_edit_dimensions_ddt.py` | Output pixel dimensions after crop+rotate |
| `tests/test_40_gallery_engine_ddt.py` | Gallery layout engine |
| `tests/test_41_thumbnail_cache_ddt.py` | Thumbnail caching |
| `tests/test_43_tag_search_ddt.py` | Tag search |
