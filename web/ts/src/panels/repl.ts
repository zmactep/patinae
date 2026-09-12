/**
 * REPL panel — command input with history and scrollable output log.
 */

import { marked } from "marked";
import DOMPurify from "dompurify";

import type { PatinaeViewer } from "../core/api.js";
import type { OutputMessage } from "../core/types.js";

export class ReplPanel {
  private container: HTMLElement;
  private viewer: PatinaeViewer;
  private output: HTMLElement;
  private input: HTMLInputElement;
  private history: string[] = [];
  private historyIdx = -1;

  constructor(container: HTMLElement, viewer: PatinaeViewer) {
    this.container = container;
    this.viewer = viewer;

    container.innerHTML = `
      <div class="repl-header">Command Line</div>
      <div class="repl-output"></div>
      <div class="repl-input-row">
        <span class="repl-prompt">Patinae&gt;</span>
        <input class="repl-input" type="text" placeholder="Type a command..." spellcheck="false" autocomplete="off" />
      </div>
    `;

    this.output = container.querySelector(".repl-output")!;
    this.input = container.querySelector(".repl-input")!;

    this.input.addEventListener("keydown", (e) => this.onKey(e));
  }

  private async onKey(e: KeyboardEvent): Promise<void> {
    if (e.key === "Enter") {
      const cmd = this.input.value.trim();
      if (!cmd) return;

      this.history.push(cmd);
      this.historyIdx = this.history.length;
      this.input.value = "";
      this.appendLine(`Patinae> ${cmd}`, "cmd");

      const result = await this.viewer.execute(cmd);
      for (const msg of result.messages) {
        this.appendOutputMessage({ level: msg.kind.toLowerCase() as OutputMessage["level"], text: msg.text, format: msg.format });
      }
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      if (this.historyIdx > 0) {
        this.historyIdx--;
        this.input.value = this.history[this.historyIdx];
      }
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      if (this.historyIdx < this.history.length - 1) {
        this.historyIdx++;
        this.input.value = this.history[this.historyIdx];
      } else {
        this.historyIdx = this.history.length;
        this.input.value = "";
      }
    }
  }

  appendOutputMessage(message: OutputMessage): void {
    if (message.level === "clear") {
      this.clearOutput();
      return;
    }
    this.appendLine(message.text, message.level, message.format);
  }

  private appendLine(text: string, level: string, format = "text"): void {
    const line = document.createElement("div");
    line.className = `repl-line repl-${level}`;
    if (format === "markdown") {
      line.classList.add("repl-markdown");
      line.innerHTML = DOMPurify.sanitize(marked.parse(text, { async: false }), {
        USE_PROFILES: { html: true },
        FORBID_TAGS: ["img", "video", "audio", "iframe", "style"],
        FORBID_ATTR: ["style"],
      });
      for (const link of line.querySelectorAll("a")) {
        link.target = "_blank";
        link.rel = "noopener noreferrer";
      }
    } else {
      line.textContent = text;
    }
    this.output.appendChild(line);
    this.output.scrollTop = this.output.scrollHeight;
  }

  private clearOutput(): void {
    this.output.replaceChildren();
  }

  update(): void {
    // REPL doesn't need periodic updates
  }

  destroy(): void {
    this.container.innerHTML = "";
  }
}
