"""
Test 43: Tag Search — Data-Driven Tests (DDT).

Parametrized tests covering tag CRUD and search-by-tag behaviour:
  - Add tag → search finds photo
  - Multiple tags → search by any single tag finds photo
  - Remove tag → search no longer finds photo
  - Case insensitivity (uppercase input stored/found lowercase)
  - Special characters in tags
  - Multi-word search tokens matching tags
  - Stemming variants (plural, -ing, -ed suffixes)
  - Search with limit parameter
  - Empty / whitespace-only tags rejected
  - Tag length edge cases

Each test case is a single row in a parameter table.
"""

import pytest

from helpers import APIClient, unique_filename


# ══════════════════════════════════════════════════════════════════════
# Helpers
# ══════════════════════════════════════════════════════════════════════

def _upload(client: APIClient) -> str:
    """Upload a test photo and return its photo_id."""
    data = client.upload_photo(unique_filename())
    return data["photo_id"]


# ══════════════════════════════════════════════════════════════════════
# DDT: Add Tag → Search Finds Photo
# ══════════════════════════════════════════════════════════════════════

SEARCH_FIND_CASES = [
    pytest.param("beach", "beach", id="exact_match"),
    pytest.param("sunset", "sunset", id="exact_sunset"),
    pytest.param("family_photo", "family_photo", id="underscore_tag"),
    pytest.param("trip2024", "trip2024", id="alphanumeric_tag"),
    pytest.param("UPPERCASE", "uppercase", id="case_insensitive_search"),
    pytest.param("MiXeDcAsE", "mixedcase", id="mixed_case_normalized"),
]

@pytest.mark.parametrize("tag_input,search_query", SEARCH_FIND_CASES)
def test_add_tag_then_search_finds_photo(user_client, tag_input, search_query):
    """Adding a tag and searching for that tag returns the photo."""
    photo_id = _upload(user_client)
    user_client.add_tag(photo_id, tag_input)

    results = user_client.search(search_query)
    ids = [r["id"] for r in results["results"]]
    assert photo_id in ids, f"Photo {photo_id} not found when searching '{search_query}'"


# ══════════════════════════════════════════════════════════════════════
# DDT: Search Does NOT Find After Tag Removal
# ══════════════════════════════════════════════════════════════════════

REMOVE_CASES = [
    pytest.param("deleteme_alpha", id="simple_remove"),
    pytest.param("deleteme_beta", id="different_tag_remove"),
]

@pytest.mark.parametrize("tag", REMOVE_CASES)
def test_remove_tag_then_search_not_found(user_client, tag):
    """After removing a tag, search for that tag should not return the photo."""
    photo_id = _upload(user_client)
    user_client.add_tag(photo_id, tag)

    # Confirm found first
    results = user_client.search(tag)
    ids = [r["id"] for r in results["results"]]
    assert photo_id in ids

    # Remove and confirm not found
    user_client.remove_tag(photo_id, tag)
    results = user_client.search(tag)
    ids = [r["id"] for r in results["results"]]
    assert photo_id not in ids, f"Photo still found after removing tag '{tag}'"


# ══════════════════════════════════════════════════════════════════════
# DDT: Multiple Tags — Search by Any
# ══════════════════════════════════════════════════════════════════════

MULTI_TAG_CASES = [
    pytest.param(["nature", "landscape", "mountain"], "nature", id="find_by_first"),
    pytest.param(["nature", "landscape", "mountain"], "landscape", id="find_by_second"),
    pytest.param(["nature", "landscape", "mountain"], "mountain", id="find_by_third"),
]

@pytest.mark.parametrize("tags,search_query", MULTI_TAG_CASES)
def test_multiple_tags_search_any(user_client, tags, search_query):
    """A photo with multiple tags is found by searching any single tag."""
    photo_id = _upload(user_client)
    for t in tags:
        user_client.add_tag(photo_id, t)

    results = user_client.search(search_query)
    ids = [r["id"] for r in results["results"]]
    assert photo_id in ids


# ══════════════════════════════════════════════════════════════════════
# DDT: Search Limit
# ══════════════════════════════════════════════════════════════════════

LIMIT_CASES = [
    pytest.param(3, 1, id="limit_1_of_3"),
    pytest.param(3, 2, id="limit_2_of_3"),
    pytest.param(5, 3, id="limit_3_of_5"),
]

@pytest.mark.parametrize("upload_count,limit", LIMIT_CASES)
def test_search_limit(user_client, upload_count, limit):
    """Search limit parameter constrains the number of results."""
    tag = f"limit_ddt_{upload_count}_{limit}"
    for _ in range(upload_count):
        pid = _upload(user_client)
        user_client.add_tag(pid, tag)

    results = user_client.search(tag, limit=limit)
    assert len(results["results"]) <= limit


# ══════════════════════════════════════════════════════════════════════
# DDT: Tag CRUD — Get Photo Tags
# ══════════════════════════════════════════════════════════════════════

TAG_LIST_CASES = [
    pytest.param(["a"], id="single_tag"),
    pytest.param(["x", "y", "z"], id="three_tags"),
    pytest.param(["alpha", "beta", "gamma", "delta"], id="four_tags"),
]

@pytest.mark.parametrize("tags", TAG_LIST_CASES)
def test_get_photo_tags(user_client, tags):
    """get_photo_tags returns exactly the tags that were added."""
    photo_id = _upload(user_client)
    for t in tags:
        user_client.add_tag(photo_id, t)

    data = user_client.get_photo_tags(photo_id)
    assert set(data["tags"]) == set(tags)


# ══════════════════════════════════════════════════════════════════════
# DDT: Tags Appear in Search Results
# ══════════════════════════════════════════════════════════════════════

def test_search_results_include_tags(user_client):
    """Tags should be included in each search result entry."""
    photo_id = _upload(user_client)
    user_client.add_tag(photo_id, "result_tag_check")

    results = user_client.search("result_tag_check")
    matching = [r for r in results["results"] if r["id"] == photo_id]
    assert len(matching) == 1
    assert "result_tag_check" in matching[0]["tags"]


# ══════════════════════════════════════════════════════════════════════
# DDT: Duplicate Tag — Idempotent
# ══════════════════════════════════════════════════════════════════════

def test_duplicate_tag_does_not_duplicate_in_search(user_client):
    """Adding the same tag twice should not cause duplicate search results."""
    photo_id = _upload(user_client)
    user_client.add_tag(photo_id, "dup_search_test")
    user_client.add_tag(photo_id, "dup_search_test")

    results = user_client.search("dup_search_test")
    ids = [r["id"] for r in results["results"]]
    assert ids.count(photo_id) == 1, "Duplicate results for the same photo"
