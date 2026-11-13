// Commands can start background work. Wait before changing the scene again.
export async function waitForTasks(viewer, reply) {
  const tasks = await Promise.all(reply.task_ids.map(id => viewer.tasks.wait(id)));
  if ("Err" in reply.result) throw new Error(reply.result.Err);
  for (const task of tasks) {
    if (task.state !== "succeeded") {
      throw new Error(`Viewer task ${task.state}: ${JSON.stringify(task.outcome)}`);
    }
  }
}

export async function executeCommand(viewer, command) {
  await waitForTasks(viewer, await viewer.execute(command));
}

export async function loadStructure(viewer, url, options) {
  await waitForTasks(viewer, await viewer.loadUrl(url, options));
}
