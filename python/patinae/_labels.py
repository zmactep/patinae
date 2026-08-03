"""Immutable Python snapshots for first-class label objects."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Iterator, Mapping, Optional, Tuple

RgbColor = Tuple[float, float, float]


def _rgb(value: Any) -> RgbColor:
    components = tuple(float(component) for component in value)
    if len(components) != 3:
        raise ValueError("label color must contain exactly three components")
    return components


@dataclass(frozen=True)
class LabelAnchor:
    """Current source-atom identity for one label entity."""

    object_name: str
    atom_index: int
    orphaned: bool
    resolved: bool

    @classmethod
    def from_mapping(cls, value: Mapping[str, Any]) -> "LabelAnchor":
        """Build an anchor snapshot from a backend mapping."""
        return cls(
            object_name=str(value["object_name"]),
            atom_index=int(value["atom_index"]),
            orphaned=bool(value["orphaned"]),
            resolved=bool(value["resolved"]),
        )


@dataclass(frozen=True)
class LabelEntity:
    """Stored text and effective presentation for one label entity."""

    anchor: LabelAnchor
    text: str
    color: RgbColor
    color_override_index: Optional[int]
    size: float
    size_override: Optional[float]
    visible: bool
    visible_override: Optional[bool]

    @property
    def object_name(self) -> str:
        """Return the source molecule name."""
        return self.anchor.object_name

    @property
    def atom_index(self) -> int:
        """Return the current zero-based source atom index."""
        return self.anchor.atom_index

    @property
    def resolved(self) -> bool:
        """Return whether the anchor currently resolves to coordinates."""
        return self.anchor.resolved

    @classmethod
    def from_mapping(cls, value: Mapping[str, Any]) -> "LabelEntity":
        """Build an entity snapshot from a backend mapping."""
        color_override = value.get("color_override_index")
        size_override = value.get("size_override")
        visible_override = value.get("visible_override")
        return cls(
            anchor=LabelAnchor.from_mapping(value["anchor"]),
            text=str(value["text"]),
            color=_rgb(value["color"]),
            color_override_index=(
                None if color_override is None else int(color_override)
            ),
            size=float(value["size"]),
            size_override=None if size_override is None else float(size_override),
            visible=bool(value["visible"]),
            visible_override=(
                None if visible_override is None else bool(visible_override)
            ),
        )


@dataclass(frozen=True)
class LabelRevisions:
    """Render-facing revision counters represented by a label snapshot."""

    geometry: int
    material: int
    labels: int

    @classmethod
    def from_mapping(cls, value: Mapping[str, Any]) -> "LabelRevisions":
        """Build revision counters from a backend mapping."""
        return cls(
            geometry=int(value["geometry"]),
            material=int(value["material"]),
            labels=int(value["labels"]),
        )


@dataclass(frozen=True)
class LabelObject:
    """Read-only snapshot of a first-class label collection."""

    name: str
    enabled: bool
    color: RgbColor
    color_override_index: Optional[int]
    size: float
    size_override: Optional[float]
    visible: bool
    visible_override: Optional[bool]
    alignment: str
    alignment_override: Optional[str]
    entities: Tuple[LabelEntity, ...]
    unresolved_count: int
    revisions: LabelRevisions

    @property
    def entity_count(self) -> int:
        """Return the number of stored label entities."""
        return len(self.entities)

    @property
    def has_unresolved_entities(self) -> bool:
        """Return whether at least one entity is unresolved."""
        return self.unresolved_count != 0

    def __len__(self) -> int:
        return len(self.entities)

    def __iter__(self) -> Iterator[LabelEntity]:
        return iter(self.entities)

    @classmethod
    def from_mapping(cls, value: Mapping[str, Any]) -> "LabelObject":
        """Build a label object snapshot from a backend mapping."""
        color_override = value.get("color_override_index")
        size_override = value.get("size_override")
        visible_override = value.get("visible_override")
        alignment_override = value.get("alignment_override")
        return cls(
            name=str(value["name"]),
            enabled=bool(value["enabled"]),
            color=_rgb(value["color"]),
            color_override_index=(
                None if color_override is None else int(color_override)
            ),
            size=float(value["size"]),
            size_override=None if size_override is None else float(size_override),
            visible=bool(value["visible"]),
            visible_override=(
                None if visible_override is None else bool(visible_override)
            ),
            alignment=str(value["alignment"]),
            alignment_override=(
                None if alignment_override is None else str(alignment_override)
            ),
            entities=tuple(
                LabelEntity.from_mapping(entity) for entity in value["entities"]
            ),
            unresolved_count=int(value["unresolved_count"]),
            revisions=LabelRevisions.from_mapping(value["revisions"]),
        )
