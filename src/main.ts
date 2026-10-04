import { invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";

interface ConvertResult {
  output_path: string;
  format: string;
  width: number;
  height: number;
  input_bytes: number;
  output_bytes: number;
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
    resultEl.textContent = "";
    if (!inputEl.value || !outputEl.value) {
      resultEl.textContent = "入力と出力を指定してください。";
      return;
    }
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
    }
  });
});
