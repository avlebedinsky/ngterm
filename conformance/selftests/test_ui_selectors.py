"""The reference's inherited palette id must not make a field or result row count as its hint."""

import runpy
from pathlib import Path

import pytest
from agterm_conformance.drivers.base import Element, _describe
from agterm_conformance.wait import WaitTimeout

palette_hint = runpy.run_path(str(Path(__file__).parents[1] / "tests/ui/conftest.py"))["palette_hint"]


class Ui:
    timeout = 0

    def __init__(self, elements):
        self.elements = elements

    def all(self, ui_id):
        return [element for element in self.elements if element.ui_id == ui_id]

    def tree_dump(self):
        return "\n".join(_describe(element) for element in self.elements)

    def text(self, element):
        return element.value or element.name or ""


def test_palette_hint_accepts_dedicated_or_inherited_text_id():
    for ui_id in ("palette-editor-hint", "command-palette"):
        read = palette_hint.__wrapped__(Ui([Element(ui_id, "text", value="Return toggles · Esc closes")]))
        assert read() == "Return toggles · Esc closes"


def test_palette_hint_does_not_hide_changed_dedicated_hint_text():
    read = palette_hint.__wrapped__(Ui([Element("palette-editor-hint", "text", value="Changed")]))
    assert read() == "Changed"  # the caller's content assertion, not a selector timeout, diagnoses it


@pytest.mark.parametrize("elements", [
    [],
    [Element("command-palette", "text-field", value="Return toggles")],
    [Element("palette-item-example", "text", value="Return toggles")],
    [Element("command-palette", "text", value="Different text")],
])
def test_palette_hint_rejects_missing_hint_fields_and_result_rows(elements):
    with pytest.raises(WaitTimeout):
        palette_hint.__wrapped__(Ui(elements))()


where_badge = runpy.run_path(str(Path(__file__).parents[1] / "tests/ui/test_keymap_commands.py"))["_where_badge"]


@pytest.mark.parametrize("elements", [
    [Element("palette-badge", "text", value="custom")],
    [Element("palette-item-custom-example", "text", value=value) for value in ("where", "custom", "ctrl+alt+k")],
])
def test_custom_command_badge_accepts_dedicated_and_inherited_ids(elements):
    assert where_badge(Ui(elements))


@pytest.mark.parametrize("elements", [
    [],
    [Element("palette-badge", "text", value="changed")],
    [Element("palette-item-custom-example", "text", value=value) for value in ("where", "changed", "ctrl+alt+k")],
    [Element("palette-item-custom-example", "text", value=value) for value in ("elsewhere", "custom", "ctrl+alt+k")],
    [Element("command-palette", "text-field", value="custom")],
    [Element("palette-item-example", "text", value=value) for value in ("where", "custom", "ctrl+alt+k")],
])
def test_custom_command_badge_rejects_changed_text_and_other_rows(elements):
    assert not where_badge(Ui(elements))
