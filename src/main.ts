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

interface PreviewImage {
  data_url: string;
  width: number;
  height: number;
  full_width: number;
  full_height: number;
}

interface MosaicRegion {
  x: number;
  y: number;
  width: number;
  height: number;
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

function autoOutput(input: string, suffix: string, ext: string): string {
  const i = Math.max(input.lastIndexOf("/"), input.lastIndexOf("\\"));
  const dir = i >= 0 ? input.slice(0, i) : "";
  const base = i >= 0 ? input.slice(i + 1) : input;
  const dot = base.lastIndexOf(".");
  const stem = dot > 0 ? base.slice(0, dot) : base;
  const sep = input.includes("\\") ? "\\" : "/";
  return `${dir}${dir ? sep : ""}${stem}_${suffix}.${ext}`;
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
  let convertManual = false;

  function refreshConvertOutput(): void {
    if (!convertManual && inputEl.value) {
      outputEl.value = autoOutput(inputEl.value, "converted", formatExtension(formatEl.value));
    }
  }

  formatEl.addEventListener("change", refreshConvertOutput);

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
      convertManual = false;
      refreshConvertOutput();
    }
  });

  pick("#btn-output").addEventListener("click", async () => {
    const ext = formatExtension(formatEl.value);
    const selected = await save({
      filters: [{ name: "画像", extensions: [ext] }],
    });
    if (typeof selected === "string") {
      outputEl.value = selected;
      convertManual = true;
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

  const resizeInputEl = pick<HTMLInputElement>("#resize-input");
  const resizeOutputEl = pick<HTMLInputElement>("#resize-output");
  const resizeFormatEl = pick<HTMLSelectElement>("#resize-format");
  const resizeQualityEl = pick<HTMLInputElement>("#resize-quality");
  const resizeQualityValueEl = pick<HTMLElement>("#resize-quality-value");
  const resizeWidthEl = pick<HTMLInputElement>("#resize-width");
  const resizeHeightEl = pick<HTMLInputElement>("#resize-height");
  const resizeResultEl = pick<HTMLElement>("#resize-result");
  let resizeManual = false;

  function resizeMode(): string {
    return (
      document.querySelector<HTMLInputElement>('input[name="resize-mode"]:checked')?.value ??
      "percent"
    );
  }

  function refreshResizeGroups(): void {
    const isPercent = resizeMode() === "percent";
    pick("#resize-percent-group").hidden = !isPercent;
    pick("#resize-size-group").hidden = isPercent;
  }

  function refreshResizeOutput(): void {
    if (!resizeManual && resizeInputEl.value) {
      resizeOutputEl.value = autoOutput(
        resizeInputEl.value,
        "resized",
        formatExtension(resizeFormatEl.value),
      );
    }
  }

  document
    .querySelectorAll<HTMLInputElement>('input[name="resize-mode"]')
    .forEach((radio) => {
      radio.addEventListener("change", refreshResizeGroups);
    });
  refreshResizeGroups();
  resizeFormatEl.addEventListener("change", refreshResizeOutput);

  const resizePercentEl = pick<HTMLInputElement>("#resize-percent");
  const resizePercentValueEl = pick<HTMLElement>("#resize-percent-value");
  resizePercentEl.addEventListener("input", () => {
    resizePercentValueEl.textContent = resizePercentEl.value;
  });

  resizeQualityEl.addEventListener("input", () => {
    resizeQualityValueEl.textContent = resizeQualityEl.value;
  });

  pick("#btn-resize-input").addEventListener("click", async () => {
    const selected = await open({
      multiple: false,
      filters: [{ name: "画像", extensions: INPUT_EXTENSIONS }],
    });
    if (typeof selected === "string") {
      resizeInputEl.value = selected;
      resizeManual = false;
      refreshResizeOutput();
    }
  });

  pick("#btn-resize-output").addEventListener("click", async () => {
    const ext = formatExtension(resizeFormatEl.value);
    const selected = await save({
      filters: [{ name: "画像", extensions: [ext] }],
    });
    if (typeof selected === "string") {
      resizeOutputEl.value = selected;
      resizeManual = true;
    }
  });

  pick("#btn-resize-apply").addEventListener("click", async () => {
    const btn = pick<HTMLButtonElement>("#btn-resize-apply");
    resizeResultEl.textContent = "";
    resizeResultEl.classList.remove("error");
    if (!resizeInputEl.value || !resizeOutputEl.value) {
      resizeResultEl.textContent = "入力と出力を指定してください。";
      resizeResultEl.classList.add("error");
      return;
    }
    btn.disabled = true;
    resizeResultEl.textContent = "リサイズ中...";
    try {
      const r = await invoke<ConvertResult>("resize_image", {
        input: resizeInputEl.value,
        output: resizeOutputEl.value,
        format: resizeFormatEl.value,
        quality: Number(resizeQualityEl.value),
        mode: resizeMode(),
        percent: Number(resizePercentEl.value),
        width: Math.max(0, Number(resizeWidthEl.value)),
        height: Math.max(0, Number(resizeHeightEl.value)),
        keepAspect: pick<HTMLInputElement>("#resize-keep").checked,
      });
      resizeResultEl.textContent =
        `${r.width}x${r.height} / ${formatKB(r.input_bytes)} → ${formatKB(r.output_bytes)} → ${r.output_path}`;
    } catch (e) {
      resizeResultEl.textContent = `リサイズに失敗しました: ${String(e)}`;
      resizeResultEl.classList.add("error");
    } finally {
      btn.disabled = false;
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

  const mosaicInputEl = pick<HTMLInputElement>("#mosaic-input");
  const mosaicOutputEl = pick<HTMLInputElement>("#mosaic-output");
  const mosaicFormatEl = pick<HTMLSelectElement>("#mosaic-format");
  const mosaicQualityEl = pick<HTMLInputElement>("#mosaic-quality");
  const mosaicQualityValueEl = pick<HTMLElement>("#mosaic-quality-value");
  const mosaicSizeEl = pick<HTMLInputElement>("#mosaic-size");
  const mosaicSizeValueEl = pick<HTMLElement>("#mosaic-size-value");
  const mosaicCanvas = pick<HTMLCanvasElement>("#mosaic-canvas");
  const mosaicInfoEl = pick<HTMLElement>("#mosaic-info");
  const mosaicResultEl = pick<HTMLElement>("#mosaic-result");
  let mosaicManual = false;

  function refreshMosaicOutput(): void {
    if (!mosaicManual && mosaicInputEl.value) {
      mosaicOutputEl.value = autoOutput(
        mosaicInputEl.value,
        "mosaic",
        formatExtension(mosaicFormatEl.value),
      );
    }
  }

  mosaicFormatEl.addEventListener("change", refreshMosaicOutput);
  const mosaicCtx = mosaicCanvas.getContext("2d");
  if (!mosaicCtx) {
    throw new Error("canvas 2d context を取得できません");
  }
  const mctx: CanvasRenderingContext2D = mosaicCtx;

  const mosaicTemp = document.createElement("canvas");
  const mosaicTempCtx = mosaicTemp.getContext("2d");
  if (!mosaicTempCtx) {
    throw new Error("canvas 2d context を取得できません");
  }
  const mtemp: CanvasRenderingContext2D = mosaicTempCtx;

  let previewImg: HTMLImageElement | null = null;
  let previewScale = 1;
  const mosaicRegions: MosaicRegion[] = [];
  let dragStart: { x: number; y: number } | null = null;
  let dragCurrent: { x: number; y: number } | null = null;
  let brushing = false;

  mosaicQualityEl.addEventListener("input", () => {
    mosaicQualityValueEl.textContent = mosaicQualityEl.value;
  });
  mosaicSizeEl.addEventListener("input", () => {
    mosaicSizeValueEl.textContent = mosaicSizeEl.value;
  });

  function mosaicTool(): string {
    return (
      document.querySelector<HTMLInputElement>('input[name="mosaic-tool"]:checked')?.value ??
      "rect"
    );
  }

  function renderMosaic(): void {
    if (!previewImg) {
      return;
    }
    const px = Math.max(2, Number(mosaicSizeEl.value));
    mctx.imageSmoothingEnabled = false;
    mctx.drawImage(previewImg, 0, 0);
    for (const r of mosaicRegions) {
      const x0 = Math.max(0, Math.round(r.x));
      const y0 = Math.max(0, Math.round(r.y));
      const w = Math.max(0, Math.round(r.width));
      const h = Math.max(0, Math.round(r.height));
      if (w < 2 || h < 2) {
        continue;
      }
      const tw = Math.max(1, Math.ceil(w / px));
      const th = Math.max(1, Math.ceil(h / px));
      mosaicTemp.width = tw;
      mosaicTemp.height = th;
      mtemp.drawImage(mosaicCanvas, x0, y0, w, h, 0, 0, tw, th);
      mctx.drawImage(mosaicTemp, 0, 0, tw, th, x0, y0, w, h);
    }
    mctx.imageSmoothingEnabled = true;
    if (dragStart && dragCurrent) {
      const x = Math.min(dragStart.x, dragCurrent.x);
      const y = Math.min(dragStart.y, dragCurrent.y);
      mctx.strokeStyle = "#3460fb";
      mctx.lineWidth = 2;
      mctx.strokeRect(x, y, Math.abs(dragStart.x - dragCurrent.x), Math.abs(dragStart.y - dragCurrent.y));
    }
    mosaicInfoEl.textContent = `${mosaicRegions.length}件の領域を選択中`;
  }

  function toPreview(e: MouseEvent): { x: number; y: number } {
    const rect = mosaicCanvas.getBoundingClientRect();
    return {
      x: ((e.clientX - rect.left) * mosaicCanvas.width) / rect.width,
      y: ((e.clientY - rect.top) * mosaicCanvas.height) / rect.height,
    };
  }

  mosaicSizeEl.addEventListener("input", () => {
    mosaicSizeValueEl.textContent = mosaicSizeEl.value;
    renderMosaic();
  });

  let lastDab: { x: number; y: number } | null = null;

  function dab(p: { x: number; y: number }): void {
    const size = Number(mosaicSizeEl.value);
    if (lastDab) {
      const dx = p.x - lastDab.x;
      const dy = p.y - lastDab.y;
      if (dx * dx + dy * dy < (size / 4) * (size / 4)) {
        return;
      }
    }
    lastDab = p;
    mosaicRegions.push({
      x: Math.round(p.x - size / 2),
      y: Math.round(p.y - size / 2),
      width: size,
      height: size,
    });
    renderMosaic();
  }

  mosaicCanvas.addEventListener("mousedown", (e) => {
    if (!previewImg) {
      return;
    }
    if (mosaicTool() === "brush") {
      brushing = true;
      lastDab = null;
      dab(toPreview(e));
    } else {
      dragStart = toPreview(e);
      dragCurrent = null;
    }
  });

  mosaicCanvas.addEventListener("mousemove", (e) => {
    if (!previewImg) {
      return;
    }
    if (brushing) {
      dab(toPreview(e));
    } else if (dragStart) {
      dragCurrent = toPreview(e);
      renderMosaic();
    }
  });

  window.addEventListener("mouseup", () => {
    brushing = false;
    lastDab = null;
    if (dragStart && dragCurrent) {
      const w = Math.abs(dragStart.x - dragCurrent.x);
      const h = Math.abs(dragStart.y - dragCurrent.y);
      if (w >= 4 && h >= 4) {
        mosaicRegions.push({
          x: Math.round(Math.min(dragStart.x, dragCurrent.x)),
          y: Math.round(Math.min(dragStart.y, dragCurrent.y)),
          width: Math.round(w),
          height: Math.round(h),
        });
      }
    }
    dragStart = null;
    dragCurrent = null;
    renderMosaic();
  });

  pick("#btn-mosaic-undo").addEventListener("click", () => {
    mosaicRegions.pop();
    renderMosaic();
  });

  pick("#btn-mosaic-clear").addEventListener("click", () => {
    mosaicRegions.length = 0;
    renderMosaic();
  });

  pick("#btn-mosaic-input").addEventListener("click", async () => {
    const selected = await open({
      multiple: false,
      filters: [{ name: "画像", extensions: INPUT_EXTENSIONS }],
    });
    if (typeof selected !== "string") {
      return;
    }
    mosaicInputEl.value = selected;
    mosaicManual = false;
    refreshMosaicOutput();
    mosaicRegions.length = 0;
    try {
      const p = await invoke<PreviewImage>("load_preview", {
        input: selected,
        maxSize: 640,
      });
      previewScale = p.full_width / p.width;
      const img = new Image();
      img.onload = () => {
        previewImg = img;
        mosaicCanvas.width = p.width;
        mosaicCanvas.height = p.height;
        renderMosaic();
      };
      img.src = p.data_url;
    } catch (e) {
      mosaicInfoEl.textContent = `プレビューの読み込みに失敗しました: ${String(e)}`;
    }
  });

  pick("#btn-mosaic-output").addEventListener("click", async () => {
    const ext = formatExtension(mosaicFormatEl.value);
    const selected = await save({
      filters: [{ name: "画像", extensions: [ext] }],
    });
    if (typeof selected === "string") {
      mosaicOutputEl.value = selected;
      mosaicManual = true;
    }
  });

  pick("#btn-mosaic-apply").addEventListener("click", async () => {
    const btn = pick<HTMLButtonElement>("#btn-mosaic-apply");
    mosaicResultEl.textContent = "";
    mosaicResultEl.classList.remove("error");
    if (!mosaicInputEl.value || !mosaicOutputEl.value) {
      mosaicResultEl.textContent = "入力と出力を指定してください。";
      mosaicResultEl.classList.add("error");
      return;
    }
    if (mosaicRegions.length === 0) {
      mosaicResultEl.textContent = "モザイク範囲を選択してください。";
      mosaicResultEl.classList.add("error");
      return;
    }
    btn.disabled = true;
    mosaicResultEl.textContent = "適用中...";
    try {
      const regions = mosaicRegions.map((r) => ({
        x: Math.round(r.x * previewScale),
        y: Math.round(r.y * previewScale),
        width: Math.round(r.width * previewScale),
        height: Math.round(r.height * previewScale),
      }));
      const r = await invoke<ConvertResult>("apply_mosaic", {
        input: mosaicInputEl.value,
        output: mosaicOutputEl.value,
        format: mosaicFormatEl.value,
        quality: Number(mosaicQualityEl.value),
        regions,
        pixelSize: Number(mosaicSizeEl.value),
      });
      mosaicResultEl.textContent =
        `${regions.length}件適用 / ${r.width}x${r.height} / ${formatKB(r.input_bytes)} → ${formatKB(r.output_bytes)} → ${r.output_path}`;
    } catch (e) {
      mosaicResultEl.textContent = `モザイク適用に失敗しました: ${String(e)}`;
      mosaicResultEl.classList.add("error");
    } finally {
      btn.disabled = false;
    }
  });
});
