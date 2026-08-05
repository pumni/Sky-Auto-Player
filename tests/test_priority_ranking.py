"""Tests for deterministic priority ranking in MetadataCoordinator."""

from pathlib import Path

from sky_music.ui.textual_app.screens.picker import MetadataPrioritySnapshot


def test_metadata_priority_snapshot_ordering() -> None:
    """Ensure MetadataPrioritySnapshot preserves strict tiered ordering without duplicates."""
    
    p1 = Path("1.txt")
    p2 = Path("2.txt")
    p3 = Path("3.txt")
    p4 = Path("4.txt")
    p5 = Path("5.txt")
    
    snapshot = MetadataPrioritySnapshot(
        selected=[p2],
        visible=[p1, p2, p3],
        overscan=[p1, p2, p3, p4],
        filtered=[p5, p4, p3, p2, p1]
    )
    
    ordered = snapshot.ordered_paths()
    
    # 1. Selected first
    assert ordered[0] == p2
    
    # 2. Visible next (p1, p3)
    assert ordered[1:3] == [p1, p3]
    
    # 3. Overscan next (p4)
    assert ordered[3] == p4
    
    # 4. Filtered last (p5)
    assert ordered[4] == p5
    
    # Length should match unique elements
    assert len(ordered) == 5
