"""Value conversion contracts exercised through the installed native module."""

import pytest

from patinae import Color, CoordSet


def test_native_coordinates_preserve_axis_order_and_report_bounds():
    coords = CoordSet([(1.25, -2.5, 8.0), (-4.0, 3.5, 2.0)])
    assert coords.get_coords() == [(1.25, -2.5, 8.0), (-4.0, 3.5, 2.0)]
    assert coords.bounding_box() == ((-4.0, -2.5, 2.0), (1.25, 3.5, 8.0))
    with pytest.raises((TypeError, ValueError)):
        CoordSet([(1.0, 2.0)])


@pytest.mark.parametrize("channels", [(255, 128, 0), (0, 1, 254)])
def test_native_color_converts_channels_across_python_boundary(channels):
    color = Color.from_rgb8(*channels)
    assert color.to_rgb8() == channels
    assert color.to_tuple() == pytest.approx(tuple(value / 255 for value in channels))


def test_native_color_rejects_invalid_input():
    with pytest.raises(ValueError, match="Invalid hex color"):
        Color.from_hex("not-a-color")
    with pytest.raises(OverflowError):
        Color.from_rgb8(256, 0, 0)
