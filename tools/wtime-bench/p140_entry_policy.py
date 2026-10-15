"""Pure validation helpers for the P140 threshold evidence contract."""
from __future__ import annotations

CONTRACT = {
    "schema_version": 1,
    "argument": "--register-entry-threshold",
    "mode": "register",
    "minimum": 1,
    "maximum": 18446744073709551615,
}
MAX_THRESHOLD = CONTRACT["maximum"]


def positive_threshold(value):
    """Return a valid positive u64, rejecting bool despite its int subclassing."""
    if type(value) is not int or not 1 <= value <= MAX_THRESHOLD:
        raise ValueError("register entry threshold must be a positive u64")
    return value


def declaration(manifest):
    """Validate optional frozen per-side execution settings; return None if absent."""
    if not isinstance(manifest, dict):
        raise ValueError("manifest must be an object")
    if "execution_settings" not in manifest:
        return None
    settings = manifest["execution_settings"]
    if not isinstance(settings, dict) or set(settings) != {"register_entry_threshold"}:
        raise ValueError("execution settings fields differ")
    threshold = settings["register_entry_threshold"]
    if not isinstance(threshold, dict) or set(threshold) != {"baseline", "candidate"}:
        raise ValueError("register threshold settings fields differ")
    if threshold["baseline"] is not None:
        raise ValueError("baseline register threshold must be null")
    candidate = positive_threshold(threshold["candidate"])
    return {"baseline": None, "candidate": candidate}


def request(manifest, mode, threshold):
    """Validate CLI intent against its frozen declaration; return per-side settings."""
    frozen = declaration(manifest)
    if threshold is None:
        if frozen is not None:
            raise ValueError("frozen register threshold declaration requires a request")
        return None
    selected = positive_threshold(threshold)
    if mode != "register":
        raise ValueError("register threshold requires register mode")
    if frozen is None:
        raise ValueError("register threshold request lacks a frozen declaration")
    if frozen["candidate"] != selected:
        raise ValueError("requested register threshold differs from frozen candidate")
    return frozen


def validate_metadata(setting, frozen):
    """Require exact JSON field types as well as the frozen per-side values."""
    observed = declaration({"execution_settings": {"register_entry_threshold": setting}})
    if observed != frozen:
        raise ValueError("adaptive entry metadata differs from frozen selection")
    return observed


def validate_wire(wire, threshold):
    """Validate a candidate raw protocol object, preserving legacy omission."""
    if not isinstance(wire, dict):
        raise ValueError("wire response must be an object")
    if threshold is None:
        if "register_entry_threshold" in wire:
            raise ValueError("unexpected register entry threshold on legacy wire")
        return wire
    selected = positive_threshold(threshold)
    if "register_entry_threshold" not in wire:
        raise ValueError("candidate wire omits register entry threshold")
    if positive_threshold(wire["register_entry_threshold"]) != selected:
        raise ValueError("candidate wire register threshold differs")
    return wire


def capability(capability):
    """Require an exact advertised candidate protocol contract."""
    if (type(capability) is not dict or capability != CONTRACT
            or any(type(capability.get(key)) is not int for key in ("schema_version", "minimum", "maximum"))):
        raise ValueError("register entry threshold capability differs")
    return capability
