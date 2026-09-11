"""Observe tasks in the current Patinae session.

Use ``from patinae import tasks`` alongside ``cmd``. Module functions resolve
the current session on each call, including the session of a running script.
For a notebook viewer, use its bound ``viewer.tasks`` interface.
"""

__all__ = ["get", "list", "cancel", "wait", "Tasks", "CommandError", "TaskError", "TaskApiError"]


class TaskApiError(RuntimeError):
    """Task API failure with a stable code, independent of the transport."""

    def __init__(self, code, message):
        super().__init__(f"{code}: {message}")
        self.code = code
        self.message = message


class CommandError(RuntimeError):
    """Command failure, retaining any background work already accepted."""

    def __init__(self, message, response):
        super().__init__(message)
        self.response = response
        self.task_ids = tuple(response["task_ids"])


class TaskError(CommandError):
    """Execution or wait failure associated with an accepted task."""

    def __init__(self, message, task_id, response, snapshot=None, *, code=None):
        super().__init__(message, response)
        self.task_id = task_id
        self.snapshot = snapshot
        self.message = message
        status = ((snapshot or {}).get("outcome") or {}).get("status") or {}
        self.code = code or (status.get("error") or {}).get("code") or status.get("status", "task_failed")


class Tasks:
    """Read, wait for, or cancel background work in this session.

    Snapshots remain in the host's bounded in-memory history. A wait timeout
    ends only the wait; cancellation is an explicit operation.
    """

    def __init__(self, backend):
        self._backend = backend

    def get(self, task_id):
        return self._call("get_task", str(task_id))

    def list(self, **filters):
        return self._call("list_tasks", filters)

    def cancel(self, task_id):
        return self._call("cancel_task", str(task_id))

    def wait(self, task_id, timeout=None):
        if timeout is not None and timeout < 0:
            raise ValueError("timeout must be nonnegative or None")
        return self._call("wait_task", str(task_id), timeout)

    def _call(self, method, *args):
        try:
            return getattr(self._backend, method)(*args)
        except TaskApiError:
            raise
        except Exception as error:
            code = getattr(error, "code", None)
            if code is None:
                raise
            raise TaskApiError(code, getattr(error, "message", str(error))) from error


def _current_tasks():
    from . import _get_cmd

    return Tasks(_get_cmd()._backend)


def get(task_id):
    """Read the retained snapshot for a task in the current session."""
    return _current_tasks().get(task_id)


def list(**filters):
    """List task snapshots matching the host's filters in the current session."""
    return _current_tasks().list(**filters)


def cancel(task_id):
    """Request cancellation of a task and its children in the current session."""
    return _current_tasks().cancel(task_id)


def wait(task_id, timeout=None):
    """Wait for a terminal snapshot; timeout in seconds ends only the wait."""
    return _current_tasks().wait(task_id, timeout)
