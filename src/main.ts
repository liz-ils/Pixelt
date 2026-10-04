import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";

interface ConvertResult {
  output_path: string;
  format: string;
  width: number;
  height: number;
  input_bytes: number;
  output_bytes: number;
}

interface BatchErrorItem {
  input: string;
  error: string;
}

interface BatchResult {
  output_dir: string;
  succeeded: ConvertResult[];
  failed: BatchErrorItem[];
}

interface BatchProgress {
  done: number;
  total: number;
  current: string;
}

const THEME_KEY = "pixelt-theme";
const THEMES = ["dads-light", "dads-dark", "contrast"] as const;

function currentTheme(): string {
  const saved = localStorage.getItem(THEME_KEY);
  if (saved && (THEMES as readonly string[]).includes(saved)) {
    return saved;
  }
  return matchMedia("(prefers-color-scheme: dark)").matches ? "dads-dark" : "dads-light";
}

function applyTheme(name: string): void {
  document.documentElement.dataset.theme = name;
  localStorage.setItem(THEME_KEY, name);
  document
    .querySelectorAll<HTMLInputElement>('input[name="theme"]')
    .forEach((radio) => {
      radio.checked = radio.value === name;
    });
}

const INPUT_EXTENSIONS = [
  "png",
  "jpg",
  "jpeg",
  "webp",
  "gif",
  "bmp",
  "tif",
  "tiff",
  "qoi",
  "ico",
];

function formatExtension(format: string): string {
  return format === "jpeg" ? "jpg" : format;
}

function formatKB(bytes: number): string {
  return `${(bytes / 1024).toFixed(1)}KB`;
}

function pick<T extends HTMLElement>(selector: string): T {
  const el = document.querySelector<T>(selector);
  if (!el) {
    throw new Error(`要素が見つかりません: ${selector}`);
  }
  return el;
}

window.addEventListener("DOMContentLoaded", () => {
  const inputEl = pick<HTMLInputElement>("#input-path");
  const outputEl = pick<HTMLInputElement>("#output-path");
  const formatEl = pick<HTMLSelectElement>("#format-select");
  const qualityEl = pick<HTMLInputElement>("#quality");
  const qualityValueEl = pick<HTMLElement>("#quality-value");
  const resultEl = pick<HTMLElement>("#result");

  qualityEl.addEventListener("input", () => {
    qualityValueEl.textContent = qualityEl.value;
  });

  applyTheme(currentTheme());
  document
    .querySelectorAll<HTMLInputElement>('input[name="theme"]')
    .forEach((radio) => {
      radio.addEventListener("change", () => applyTheme(radio.value));
    });

  const batchFormatEl = pick<HTMLSelectElement>("#batch-format");
  const batchQualityEl = pick<HTMLInputElement>("#batch-quality");
  const batchQualityValueEl = pick<HTMLElement>("#batch-quality-value");
  batchQualityEl.addEventListener("input", () => {
    batchQualityValueEl.textContent = batchQualityEl.value;
  });

  pick("#btn-input").addEventListener("click", async () => {
    const selected = await open({
      multiple: false,
      filters: [{ name: "画像", extensions: INPUT_EXTENSIONS }],
    });
    if (typeof selected === "string") {
      inputEl.value = selected;
    }
  });

  pick("#btn-output").addEventListener("click", async () => {
    const ext = formatExtension(formatEl.value);
    const selected = await save({
      filters: [{ name: "画像", extensions: [ext] }],
    });
    if (typeof selected === "string") {
      outputEl.value = selected;
    }
  });

  pick("#btn-convert").addEventListener("click", async () => {
    const btn = pick<HTMLButtonElement>("#btn-convert");
    resultEl.textContent = "";
    resultEl.classList.remove("error");
    if (!inputEl.value || !outputEl.value) {
      resultEl.textContent = "入力と出力を指定してください。";
      resultEl.classList.add("error");
      return;
    }
    btn.disabled = true;
    resultEl.textContent = "変換中...";
    try {
      const r = await invoke<ConvertResult>("convert_image", {
        input: inputEl.value,
        output: outputEl.value,
        format: formatEl.value,
        quality: Number(qualityEl.value),
      });
      const ratio = ((r.output_bytes / r.input_bytes) * 100).toFixed(1);
      resultEl.textContent =
        `${r.width}x${r.height} / ${formatKB(r.input_bytes)} → ${formatKB(r.output_bytes)} (${ratio}%) → ${r.output_path}`;
    } catch (e) {
      resultEl.textContent = `変換に失敗しました: ${String(e)}`;
      resultEl.classList.add("error");
    } finally {
      btn.disabled = false;
    }
  });

  let batchInputs: string[] = [];
  const batchDirEl = pick<HTMLInputElement>("#batch-dir");
  const batchInfoEl = pick<HTMLElement>("#batch-info");
  const batchResultEl = pick<HTMLElement>("#batch-result");

  pick("#btn-batch-inputs").addEventListener("click", async () => {
    const selected = await open({
      multiple: true,
      filters: [{ name: "画像", extensions: INPUT_EXTENSIONS }],
    });
    if (Array.isArray(selected)) {
      batchInputs = selected;
      batchInfoEl.textContent = `${batchInputs.length}件選択`;
    }
  });

  pick("#btn-batch-dir").addEventListener("click", async () => {
    const selected = await open({ multiple: false, directory: true });
    if (typeof selected === "string") {
      batchDirEl.value = selected;
    }
  });

  pick("#btn-batch").addEventListener("click", async () => {
    const btn = pick<HTMLButtonElement>("#btn-batch");
    const progressEl = pick<HTMLProgressElement>("#batch-progress");
    const statusEl = pick<HTMLElement>("#batch-status");
    batchResultEl.textContent = "";
    batchResultEl.classList.remove("error");
    if (batchInputs.length === 0 || !batchDirEl.value) {
      batchResultEl.textContent = "入力ファイルと出力フォルダを指定してください。";
      batchResultEl.classList.add("error");
      return;
    }
    btn.disabled = true;
    progressEl.hidden = false;
    progressEl.value = 0;
    statusEl.textContent = `0/${batchInputs.length}件処理中...`;
    const unlisten = await listen<BatchProgress>("batch-progress", (e) => {
      progressEl.value = (e.payload.done / e.payload.total) * 100;
      statusEl.textContent =
        `${e.payload.done}/${e.payload.total}件処理中: ${e.payload.current}`;
    });
    try {
      const r = await invoke<BatchResult>("convert_batch", {
        inputs: batchInputs,
        outputDir: batchDirEl.value,
        format: batchFormatEl.value,
        quality: Number(batchQualityEl.value),
        overwrite: pick<HTMLInputElement>("#overwrite").checked,
      });
      const lines = [
        `成功 ${r.succeeded.length}件 / 失敗 ${r.failed.length}件 → ${r.output_dir}`,
        ...r.failed.map((f) => `失敗: ${f.input}: ${f.error}`),
      ];
      batchResultEl.textContent = lines.join("\n");
      if (r.failed.length > 0) {
        batchResultEl.classList.add("error");
      }
      statusEl.textContent = "完了";
    } catch (e) {
      batchResultEl.textContent = `一括変換に失敗しました: ${String(e)}`;
      batchResultEl.classList.add("error");
      statusEl.textContent = "";
    } finally {
      unlisten();
      btn.disabled = false;
      progressEl.hidden = true;
    }
  });

  document
    .querySelectorAll<HTMLButtonElement>(".nav-item[data-target]")
    .forEach((btn) => {
      btn.addEventListener("click", () => {
        document
          .querySelectorAll(".nav-item")
          .forEach((b) => b.classList.remove("active"));
        btn.classList.add("active");
        document
          .querySelectorAll(".view")
          .forEach((v) => v.classList.remove("active"));
        document.getElementById(btn.dataset.target ?? "")?.classList.add("active");
      });
    });
});
