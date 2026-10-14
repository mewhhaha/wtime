"""Fixed P127 fuel policy and ten-row staged-workload model draft."""
from __future__ import annotations
import hashlib, json
from pathlib import Path
from typing import Any

REGISTRY_SHA256 = "8917f14d06b2dd69d1abb1885d8186cdc11191924aef8ee073e356cec5cfd668"
POLICIES = (("unmetered", None), ("metered", 50_000_000))
MODES = ("decoded", "compact")
STAGES = ("prepare", "instantiate", "first_call", "warm_call", "load_and_call")
INPUTS = (0, 17, 32, 10000)
SCALAR_IDS = tuple(f"scalar_{family}_n{n}" for family in ("accumulator", "dependent") for n in INPUTS)
ROW_IDS = (*SCALAR_IDS, "call_overhead", "prepare_large")
WARM_BATCH = 3

class P127ModelError(ValueError): pass

def sha256(path: Path) -> str:
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()

def policy(name_or_value: Any) -> dict[str, Any]:
    """Return an exact accepted policy; bools are never integer budgets."""
    if name_or_value is None or name_or_value == "unmetered":
        return {"name": "unmetered", "source_fuel": None}
    if name_or_value == "metered" or (type(name_or_value) is int and name_or_value == 50_000_000):
        return {"name": "metered", "source_fuel": 50_000_000}
    raise P127ModelError(f"unsupported source-fuel policy: {name_or_value!r}")

def protocol_contract() -> dict[str, Any]:
    return {"schema_version": 1, "accepted": [
        {"name": "unmetered", "source_fuel": None},
        {"name": "metered", "source_fuel": 50_000_000},
    ]}

def parse_cli(value: str) -> dict[str, Any]:
    if value == "none": return policy("unmetered")
    if value == "50000000": return policy("metered")
    raise P127ModelError(f"unsupported --source-fuel value: {value!r}")

def cli_value(setting: dict[str, Any]) -> str:
    if not isinstance(setting, dict) or set(setting) != {"name", "source_fuel"}:
        raise P127ModelError("policy requires exact name and source_fuel fields")
    checked = policy(setting.get("name"))
    budget = setting["source_fuel"]
    if setting != checked or (budget is not None and type(budget) is not int):
        raise P127ModelError("policy name and typed source_fuel disagree")
    return "none" if checked["source_fuel"] is None else str(checked["source_fuel"])

def validate_wire_setting(raw: dict[str, Any], setting: dict[str, Any]) -> None:
    cli_value(setting)
    if "source_fuel" not in raw or "fuel_policy" not in raw:
        raise P127ModelError("observation lacks explicit fuel settings")
    budget = raw["source_fuel"]
    if (raw["fuel_policy"] != setting["name"] or budget != setting["source_fuel"]
            or (budget is not None and type(budget) is not int)):
        raise P127ModelError("observation fuel policy or budget differs")

def expected_result(row: dict[str, Any]) -> Any:
    """Independent scalar oracle, including explicit prepare-only null."""
    wid = row.get("id")
    if wid == "prepare_large": return None
    if wid == "call_overhead": return 101
    parts = wid.split("_")
    if len(parts) != 3 or parts[0] != "scalar" or parts[1] not in {"accumulator", "dependent"}:
        raise P127ModelError(f"unexpected P127 row {wid!r}")
    try: n = int(parts[2][1:])
    except (ValueError, IndexError) as exc: raise P127ModelError(f"malformed row id {wid!r}") from exc
    if parts[2][0] != "n" or n not in INPUTS: raise P127ModelError(f"unexpected scalar input: {wid}")
    if parts[1] == "accumulator": return n * (n - 1) // 2
    # scalar_workloads.py's dependent recurrence uses the LCG update n times.
    value = 1
    for _ in range(n): value = (value * 1664525 + 1013904223) & 0xFFFFFFFF
    return value - 0x100000000 if value & 0x80000000 else value

def _stages(row: dict[str, Any]) -> tuple[str, ...]:
    stages = row.get("stages")
    if row.get("id") == "prepare_large": wanted = ("prepare",)
    else: wanted = STAGES
    if not isinstance(stages, list) or tuple(stages) != wanted:
        raise P127ModelError(f"frozen stages differ for {row.get('id')}: {stages!r}")
    return wanted

def entries(registry_path: Path) -> list[dict[str, Any]]:
    path = Path(registry_path)
    raw = path.read_bytes()
    if hashlib.sha256(raw).hexdigest() != REGISTRY_SHA256:
        raise P127ModelError("shared workload registry hash changed")
    registry = json.loads(raw)
    rows = registry.get("workloads")
    if not isinstance(rows, list) or len(rows) != 459:
        raise P127ModelError("P127 requires the frozen 459-row registry")
    by_id = {r.get("id"): r for r in rows}
    if len(by_id) != 459 or any(wid not in by_id for wid in ROW_IDS):
        raise P127ModelError("P127 selection row missing or duplicated")
    selected = []
    for wid in ROW_IDS:
        row = by_id[wid]
        _stages(row)
        if wid.startswith("scalar_"):
            param = row.get("params", [{}])[0]
            expected_n = int(wid.rsplit("n", 1)[1])
            if not (param.get("min") == expected_n == param.get("max") == param.get("baseline")):
                raise P127ModelError(f"input declaration drift for {wid}")
        selected.append({"id": wid, "source": row["source"], "params": row["params"],
                         "result": row["result"], "stages": list(_stages(row)),
                         "expected": expected_result(row), "input": row["params"][0]["baseline"] if row.get("params") else None})
    if len(selected) != 10: raise P127ModelError("P127 selection must contain exactly ten rows")
    return selected

def matrix(rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    out=[]
    for row in rows:
        for stage in row["stages"]:
            for mode in MODES:
                for name, fuel in POLICIES:
                    out.append({"workload":row["id"],"stage":stage,"mode":mode,
                                "fuel_policy":name,"source_fuel":fuel,
                                "batch":WARM_BATCH if stage == "warm_call" else 1,
                                "input":row["input"],
                                "expected":row["expected"] if stage != "prepare" or row["id"] == "prepare_large" else row["expected"],
                                "result":None if row["id"] == "prepare_large" else row["expected"]})
    if len(out)!=184: raise P127ModelError(f"expected 184 tuples, got {len(out)}")
    return out
