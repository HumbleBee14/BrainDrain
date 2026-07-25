"""Activity results read by attribute must declare result_type.

Temporal only rebuilds a dataclass result when it knows the target type. Calling
an activity by string name without result_type hands the workflow a plain dict,
so `result.field` raises AttributeError at runtime — and inside a try/except it
surfaces as a vague failure with the real cause buried.

Primitive results (str, None) need no result_type: there is no dataclass to
rebuild. So the rule is scoped to results the workflow actually reads fields off.
"""

import ast
from pathlib import Path

import pytest

_WORKFLOW_DIR = Path(__file__).resolve().parents[1] / "src" / "workflows"


def _attribute_reads(tree: ast.AST) -> set[str]:
    """Names that appear as `name.something` anywhere in the tree."""
    return {
        node.value.id
        for node in ast.walk(tree)
        if isinstance(node, ast.Attribute) and isinstance(node.value, ast.Name)
    }


def _assigned_name(node: ast.AST) -> str | None:
    if isinstance(node, ast.Assign) and len(node.targets) == 1:
        target = node.targets[0]
        return target.id if isinstance(target, ast.Name) else None
    if isinstance(node, ast.AnnAssign) and isinstance(node.target, ast.Name):
        return node.target.id
    return None


def _is_string_named_activity(call: ast.AST) -> bool:
    return (
        isinstance(call, ast.Call)
        and isinstance(call.func, ast.Attribute)
        and call.func.attr == "execute_activity"
        and bool(call.args)
        and isinstance(call.args[0], ast.Constant)
        and isinstance(call.args[0].value, str)
    )


def _untyped_field_reads(tree: ast.AST) -> list[str]:
    """Activity results that are read by attribute but carry no result_type."""
    read_by_attribute = _attribute_reads(tree)
    offenders = []

    for node in ast.walk(tree):
        name = _assigned_name(node)
        if name is None or name not in read_by_attribute:
            continue

        call = node.value
        if isinstance(call, ast.Await):
            call = call.value
        if not _is_string_named_activity(call):
            continue

        if "result_type" not in {kw.arg for kw in call.keywords}:
            offenders.append(f"{name} = {call.args[0].value}() at line {call.lineno}")

    return offenders


def _workflow_files():
    return sorted(p for p in _WORKFLOW_DIR.glob("*.py") if p.name != "__init__.py")


@pytest.mark.parametrize("path", _workflow_files(), ids=lambda p: p.name)
def test_activity_results_read_by_attribute_declare_result_type(path):
    offenders = _untyped_field_reads(ast.parse(path.read_text()))
    assert not offenders, (
        f"{path.name}: attribute access on an untyped activity result:\n" + "\n".join(offenders)
    )


def test_detects_the_bug_it_guards_against():
    """The ingest.py regression: annotation alone does not make Temporal rebuild."""
    bad = ast.parse(
        "async def f():\n"
        "    info: DocumentInfo = await workflow.execute_activity('get_document_info', X())\n"
        "    return info.status\n"
    )
    assert _untyped_field_reads(bad) == ["info = get_document_info() at line 2"]


def test_allows_primitive_and_typed_results():
    ok = ast.parse(
        "async def f():\n"
        "    job_id = await workflow.execute_activity('create_job', X())\n"
        "    res = await workflow.execute_activity('chunk', X(), result_type=Out)\n"
        "    return job_id, res.count\n"
    )
    assert _untyped_field_reads(ok) == []
