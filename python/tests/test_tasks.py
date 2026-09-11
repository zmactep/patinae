"""Task error contracts across Python backends."""
import pytest
from patinae import Cmd
from patinae.tasks import Tasks, TaskApiError, TaskError

class FailingBackend:
    def __init__(self, code):
        self.code = code

    def fail(self, *args):
        error = RuntimeError("transport presentation")
        error.code = self.code
        error.message = "host detail"
        raise error

    get_task = list_tasks = cancel_task = wait_task = fail

    def execute(self, command, quiet=False):
        return {"result": {"Ok": None}, "messages": [], "task_ids": ["accepted-id"]}

@pytest.mark.parametrize("code", ["wrong_executor", "timeout", "apply_failed", "expired"])
@pytest.mark.parametrize("method,args", [("get", ("id",)), ("list", ()), ("cancel", ("id",)), ("wait", ("id",))])
def test_task_api_preserves_host_code_and_message(code, method, args):
    with pytest.raises(TaskApiError) as caught:
        getattr(Tasks(FailingBackend(code)), method)(*args)
    assert caught.value.code == code
    assert caught.value.message == "host detail"

def test_command_wait_preserves_receipt_and_code():
    with pytest.raises(TaskError) as caught:
        Cmd(FailingBackend("timeout")).fetch("1crn")
    assert caught.value.code == "timeout"
    assert caught.value.task_id == "accepted-id"
    assert caught.value.response["task_ids"] == ["accepted-id"]

def test_terminal_failure_keeps_snapshot_code():
    snapshot = {"state": "failed", "outcome": {"status": {"status": "failure", "error": {"code": "apply_failed", "message": "detail"}}}}
    error = TaskError("detail", "id", {"task_ids": ["id"]}, snapshot)
    assert error.code == "apply_failed"
    assert error.snapshot is snapshot


@pytest.mark.parametrize("method", ["get", "cancel", "wait"])
def test_native_binding_preserves_invalid_id_through_public_api(method):
    from patinae._patinae import _create_standalone_backend

    tasks = Tasks(_create_standalone_backend())
    with pytest.raises(TaskApiError) as caught:
        getattr(tasks, method)("not-an-id")
    assert caught.value.code == "invalid_id"
    assert caught.value.message == "invalid_id"


def test_native_inline_and_pml_commands_share_registered_syntax(tmp_path):
    from patinae._patinae import _create_standalone_backend

    backend = _create_standalone_backend()
    cmd = Cmd(backend)
    tasks = Tasks(backend)
    source = r'python assert r"a\ b" == "a\\ b"; from patinae import cmd; cmd.do("group inline_source")'
    reply = cmd.do(source)
    assert tasks.get(reply["task_ids"][0])["state"] == "succeeded"

    script = tmp_path / "source.pml"
    script.write_text(source.replace("inline_source", "pml_source") + "\ngroup after_source\n")
    reply = cmd.run(str(script))
    assert tasks.get(reply["task_ids"][0])["state"] == "succeeded"
    assert {"inline_source", "pml_source", "after_source"} <= set(backend.get_names())
