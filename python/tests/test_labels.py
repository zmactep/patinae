"""Python API coverage for first-class label objects."""

from dataclasses import FrozenInstanceError

import pytest

from patinae import Cmd, LabelObject


class RecordingBackend:
    def __init__(self, label=None):
        self.commands = []
        self.label_snapshot = label

    def execute(self, command, silent=False):
        self.commands.append((command, silent))

    def get_label(self, name):
        if self.label_snapshot is None or self.label_snapshot["name"] != name:
            raise KeyError(name)
        return self.label_snapshot


def label_snapshot():
    return {
        "name": "ca_labels",
        "enabled": True,
        "color": [0.0, 1.0, 1.0],
        "color_override_index": None,
        "size": 18.0,
        "size_override": 18.0,
        "visible": True,
        "visible_override": None,
        "alignment": "bottom-left",
        "alignment_override": None,
        "entities": [
            {
                "anchor": {
                    "object_name": "protein",
                    "atom_index": 4,
                    "orphaned": False,
                    "resolved": True,
                },
                "text": "ALA1",
                "color": [1.0, 0.0, 0.0],
                "color_override_index": 2,
                "size": 20.0,
                "size_override": 20.0,
                "visible": False,
                "visible_override": False,
            },
            {
                "anchor": {
                    "object_name": "deleted",
                    "atom_index": 8,
                    "orphaned": True,
                    "resolved": False,
                },
                "text": "orphaned",
                "color": [0.0, 1.0, 1.0],
                "color_override_index": None,
                "size": 18.0,
                "size_override": None,
                "visible": True,
                "visible_override": None,
            },
        ],
        "unresolved_count": 1,
        "revisions": {"geometry": 3, "material": 4, "labels": 5},
    }


def test_label_builds_auto_named_command():
    backend = RecordingBackend()
    cmd = Cmd(backend)

    cmd.label("name CA", "resi")

    assert backend.commands == [("label name CA, resi", True)]


def test_label_builds_named_append_command_and_preserves_expression():
    backend = RecordingBackend()
    cmd = Cmd(backend)

    cmd.label("chain A", '"active site"', object="notes", quiet=False)

    assert backend.commands == [
        ('label chain A, "active site", object=notes', False)
    ]


def test_get_label_returns_immutable_typed_snapshot():
    cmd = Cmd(RecordingBackend(label_snapshot()))

    label = cmd.get_label("ca_labels")

    assert isinstance(label, LabelObject)
    assert label.entity_count == 2
    assert label.has_unresolved_entities
    assert label.revisions.labels == 5
    assert label.entities[0].object_name == "protein"
    assert label.entities[0].atom_index == 4
    assert label.entities[0].color == (1.0, 0.0, 0.0)
    assert label.entities[0].visible_override is False
    assert not label.entities[1].resolved
    with pytest.raises(FrozenInstanceError):
        label.name = "renamed"
