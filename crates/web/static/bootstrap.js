const EXPECTED_ORIGIN = "http://127.0.0.1:4173";
const status = document.querySelector("#status");
const DATABASE_NAME = "eden-web";
const DATABASE_VERSION = 1;
const DATABASE_STORES = [
  "handles",
  "settings",
  "progress",
  "readflags",
  "cgseen",
  "saves",
];
let databasePromise;

function requestResult(request) {
  return new Promise((resolve, reject) => {
    request.addEventListener("success", () => resolve(request.result), { once: true });
    request.addEventListener(
      "error",
      () => reject(request.error ?? new Error("IndexedDB request failed")),
      { once: true },
    );
  });
}

function transactionDone(transaction) {
  return new Promise((resolve, reject) => {
    transaction.addEventListener("complete", () => resolve(), { once: true });
    transaction.addEventListener(
      "abort",
      () => reject(transaction.error ?? new Error("IndexedDB transaction aborted")),
      { once: true },
    );
    transaction.addEventListener(
      "error",
      () => reject(transaction.error ?? new Error("IndexedDB transaction failed")),
      { once: true },
    );
  });
}

function openDatabase() {
  if (!databasePromise) {
    databasePromise = new Promise((resolve, reject) => {
      const request = indexedDB.open(DATABASE_NAME, DATABASE_VERSION);
      request.addEventListener("upgradeneeded", () => {
        const database = request.result;
        for (const store of DATABASE_STORES) {
          if (!database.objectStoreNames.contains(store)) {
            database.createObjectStore(store);
          }
        }
      });
      request.addEventListener("success", () => {
        const database = request.result;
        database.addEventListener("versionchange", () => {
          database.close();
          databasePromise = undefined;
        });
        resolve(database);
      }, { once: true });
      request.addEventListener(
        "error",
        () => reject(request.error ?? new Error("Cannot open IndexedDB")),
        { once: true },
      );
      request.addEventListener(
        "blocked",
        () => reject(new Error("IndexedDB upgrade is blocked by another eden tab")),
        { once: true },
      );
    });
  }
  return databasePromise;
}

globalThis.edenStorageLoadAll = async () => {
  const database = await openDatabase();
  const records = [];
  for (const storeName of DATABASE_STORES.filter((name) => name !== "handles")) {
    const transaction = database.transaction(storeName, "readonly");
    const store = transaction.objectStore(storeName);
    const done = transactionDone(transaction);
    const [keys, values] = await Promise.all([
      requestResult(store.getAllKeys()),
      requestResult(store.getAll()),
    ]);
    await done;
    for (let index = 0; index < keys.length; index += 1) {
      records.push({ store: storeName, key: String(keys[index]), value: values[index] });
    }
  }
  return records;
};

globalThis.edenStoragePut = async (storeName, key, value) => {
  if (!DATABASE_STORES.includes(storeName)) {
    throw new Error(`Unknown eden object store: ${storeName}`);
  }
  const database = await openDatabase();
  const transaction = database.transaction(storeName, "readwrite");
  transaction.objectStore(storeName).put(value, key);
  await transactionDone(transaction);
};

globalThis.edenStoragePutMany = async (storeName, records) => {
  if (!DATABASE_STORES.includes(storeName)) {
    throw new Error(`Unknown eden object store: ${storeName}`);
  }
  const database = await openDatabase();
  const transaction = database.transaction(storeName, "readwrite");
  const store = transaction.objectStore(storeName);
  for (const record of records) {
    store.put(record.value, record.key);
  }
  await transactionDone(transaction);
};

globalThis.edenStorageDelete = async (storeName, key) => {
  const database = await openDatabase();
  const transaction = database.transaction(storeName, "readwrite");
  transaction.objectStore(storeName).delete(key);
  await transactionDone(transaction);
};

globalThis.edenStorageClear = async () => {
  if (databasePromise) {
    const database = await databasePromise;
    database.close();
    databasePromise = undefined;
  }
  await requestResult(indexedDB.deleteDatabase(DATABASE_NAME));
};

globalThis.edenHandleSave = async (handle) => {
  await globalThis.edenStoragePut("handles", "game-root", handle);
};

globalThis.edenHandleLoad = async () => {
  const database = await openDatabase();
  const transaction = database.transaction("handles", "readonly");
  const done = transactionDone(transaction);
  const handle = await requestResult(transaction.objectStore("handles").get("game-root"));
  await done;
  return handle;
};

globalThis.edenPickSaveFile = async () => {
  const [handle] = await window.showOpenFilePicker({
    id: "eden-import-save",
    multiple: false,
    types: [{
      description: "eden 存档",
      accept: { "application/octet-stream": [".sav"] },
    }],
  });
  const file = await handle.getFile();
  return {
    name: file.name,
    bytes: new Uint8Array(await file.arrayBuffer()),
  };
};

globalThis.edenChooseImportSlot = (fileName, suggestedSlot) => new Promise((resolve) => {
  const dialog = document.getElementById("import-save-dialog");
  const name = document.getElementById("import-save-name");
  const input = document.getElementById("import-save-slot");
  if (!(dialog instanceof HTMLDialogElement)
      || !(input instanceof HTMLInputElement)
      || dialog.open) {
    resolve(null);
    return;
  }
  name.textContent = fileName;
  input.value = String(Math.min(99, Math.max(10, suggestedSlot)));
  dialog.addEventListener("close", () => {
    const slot = Number.parseInt(input.value, 10);
    resolve(dialog.returnValue === "ok" && slot >= 10 && slot <= 99 ? slot : null);
  }, { once: true });
  dialog.showModal();
  input.focus();
  input.select();
});

globalThis.edenConfirmSaveOverwrite = (slot) =>
  window.confirm(`槽位 ${slot} 已有存档，确定覆盖吗？`);

globalThis.edenDownloadBytes = (fileName, mimeType, bytes) => {
  const blob = new Blob([bytes], { type: mimeType });
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = url;
  link.download = fileName;
  link.hidden = true;
  document.body.append(link);
  link.click();
  link.remove();
  setTimeout(() => URL.revokeObjectURL(url), 60_000);
};

globalThis.edenCaptureThumbnail = async (canvas) => {
  await new Promise((resolve) => requestAnimationFrame(resolve));
  await new Promise((resolve) => requestAnimationFrame(resolve));
  const thumbnail = document.createElement("canvas");
  thumbnail.width = 128;
  thumbnail.height = 80;
  const context = thumbnail.getContext("2d", { alpha: false });
  if (!context) throw new Error("无法创建缩略图 2D Canvas");
  context.drawImage(canvas, 0, 0, thumbnail.width, thumbnail.height);
  const blob = await new Promise((resolve, reject) => {
    thumbnail.toBlob(
      (value) => value ? resolve(value) : reject(new Error("缩略图 PNG 编码失败")),
      "image/png",
    );
  });
  return new Uint8Array(await blob.arrayBuffer());
};

globalThis.edenShowContextMenu = (state) => new Promise((resolve) => {
  const dialog = document.getElementById("context-menu-dialog");
  if (!(dialog instanceof HTMLDialogElement) || dialog.open) {
    resolve(null);
    return;
  }
  for (const button of dialog.querySelectorAll("button[data-command]")) {
    const command = button.dataset.command;
    button.disabled =
      (["quick-save", "save", "load"].includes(command) && !state.saveLoadEnabled)
      || (command === "toggle-panel" && !state.panelToggleEnabled)
      || (command === "fullscreen" && state.fullscreen)
      || (command === "windowed" && !state.fullscreen);
    if (command === "toggle-panel") {
      button.textContent = state.panelHidden ? "显示消息框" : "隐藏消息框";
    } else if (command === "auto") {
      button.textContent = `${state.autoChecked ? "✓ " : ""}自动播放`;
    } else if (command === "skip") {
      button.textContent = `${state.skipChecked ? "✓ " : ""}跳过已读`;
    }
  }
  dialog.addEventListener("close", () => {
    const value = dialog.returnValue;
    resolve(value && value !== "cancel" ? value : null);
  }, { once: true });
  dialog.showModal();
});

globalThis.edenEditSaveComment = (initial, maxChars) => new Promise((resolve) => {
  const dialog = document.getElementById("save-comment-dialog");
  const input = document.getElementById("save-comment-input");
  if (!(dialog instanceof HTMLDialogElement) || !(input instanceof HTMLInputElement) || dialog.open) {
    resolve(null);
    return;
  }
  input.value = initial;
  input.maxLength = maxChars;
  dialog.addEventListener("close", () => {
    resolve(dialog.returnValue === "ok" ? input.value : null);
  }, { once: true });
  dialog.showModal();
  input.focus();
  input.select();
});

globalThis.edenAudioCreate = () => {
  const AudioContextClass = globalThis.AudioContext ?? globalThis.webkitAudioContext;
  if (!AudioContextClass) {
    throw new Error("WebAudio AudioContext unavailable");
  }
  return { context: new AudioContextClass(), channels: new Map() };
};

globalThis.edenAudioResume = async (audio) => {
  try {
    if (audio.context.state === "suspended") {
      await audio.context.resume();
    }
  } catch (error) {
    console.warn("WebAudio resume failed", error);
  }
};

globalThis.edenAudioPlay = async (
  audio,
  channel,
  bytes,
  name,
  volume,
  pan,
  looping,
  fadeSeconds,
) => {
  globalThis.edenAudioStop(audio, channel, 0);
  const generation = (audio.channels.get(channel)?.generation ?? 0) + 1;
  const slot = { generation, name, status: "decoding", source: null, gain: null, panner: null, analyser: null };
  audio.channels.set(channel, slot);
  try {
    const copy = bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
    const buffer = await audio.context.decodeAudioData(copy);
    if (audio.channels.get(channel) !== slot) return;
    const source = audio.context.createBufferSource();
    const gain = audio.context.createGain();
    const panner = audio.context.createStereoPanner();
    const analyser = audio.context.createAnalyser();
    analyser.fftSize = 1024;
    source.buffer = buffer;
    source.loop = looping;
    panner.pan.value = Math.max(-1, Math.min(1, pan));
    const targetGain = Math.max(0, Math.min(1, volume));
    gain.gain.setValueAtTime(fadeSeconds > 0 ? 0 : targetGain, audio.context.currentTime);
    if (fadeSeconds > 0) {
      gain.gain.linearRampToValueAtTime(targetGain, audio.context.currentTime + fadeSeconds);
    }
    source.connect(gain).connect(panner).connect(analyser).connect(audio.context.destination);
    Object.assign(slot, {
      status: "playing",
      source,
      gain,
      panner,
      analyser,
      startedAt: audio.context.currentTime,
      duration: buffer.duration,
      looping,
    });
    source.addEventListener("ended", () => {
      if (audio.channels.get(channel) === slot) slot.status = "ended";
    }, { once: true });
    source.start();
  } catch (error) {
    if (audio.channels.get(channel) === slot) {
      slot.status = "failed";
      slot.error = error instanceof Error ? error.message : String(error);
      console.error(`WebAudio decode failed ${channel}:${name}`, error);
    }
  }
};

globalThis.edenAudioSet = (audio, channel, volume, pan) => {
  const slot = audio.channels.get(channel);
  if (!slot) return;
  if (slot.gain) slot.gain.gain.setValueAtTime(Math.max(0, Math.min(1, volume)), audio.context.currentTime);
  if (slot.panner) slot.panner.pan.setValueAtTime(Math.max(-1, Math.min(1, pan)), audio.context.currentTime);
};

globalThis.edenAudioStop = (audio, channel, fadeSeconds) => {
  const slot = audio.channels.get(channel);
  if (!slot) return;
  audio.channels.delete(channel);
  if (!slot.source) return;
  const stopAt = audio.context.currentTime + Math.max(0, fadeSeconds);
  try {
    if (slot.gain && fadeSeconds > 0) {
      slot.gain.gain.cancelScheduledValues(audio.context.currentTime);
      slot.gain.gain.setValueAtTime(slot.gain.gain.value, audio.context.currentTime);
      slot.gain.gain.linearRampToValueAtTime(0, stopAt);
    }
    slot.source.stop(stopAt);
  } catch (_) {
    // Already stopped nodes are harmless.
  }
};

globalThis.edenAudioPlaying = (audio, channel) => {
  const status = audio.channels.get(channel)?.status;
  return status === "decoding" || status === "playing";
};

globalThis.edenAudioElapsed = (audio, channel) => {
  const slot = audio.channels.get(channel);
  return slot?.status === "playing" ? Math.max(0, audio.context.currentTime - slot.startedAt) : -1;
};

globalThis.edenAudioVoiceLevel = (audio) => {
  const analyser = audio.channels.get("voice")?.analyser;
  if (!analyser) return 0;
  const samples = new Uint8Array(analyser.fftSize);
  analyser.getByteTimeDomainData(samples);
  let peak = 0;
  for (const sample of samples) peak = Math.max(peak, Math.abs(sample - 128) / 128);
  return peak;
};

globalThis.edenAudioSpectrum = (audio) => {
  const analyser = audio.channels.get("bgm")?.analyser;
  if (!analyser) return new Float32Array(20);
  const bins = new Uint8Array(analyser.frequencyBinCount);
  analyser.getByteFrequencyData(bins);
  const bands = new Float32Array(20);
  for (let band = 0; band < bands.length; band += 1) {
    const begin = Math.floor(band * bins.length / bands.length);
    const end = Math.max(begin + 1, Math.floor((band + 1) * bins.length / bands.length));
    let peak = 0;
    for (let index = begin; index < end; index += 1) peak = Math.max(peak, bins[index]);
    bands[band] = peak / 255;
  }
  return bands;
};

function showError(message) {
  status.className = "status status--error";
  status.textContent = message;
}

function capabilityErrors() {
  const errors = [];
  if (window.location.origin !== EXPECTED_ORIGIN) {
    errors.push(`请使用固定地址 ${EXPECTED_ORIGIN}`);
  }
  if (!("gpu" in navigator)) {
    errors.push("当前浏览器未提供 WebGPU");
  }
  if (!("showDirectoryPicker" in window)) {
    errors.push("当前浏览器未提供 File System Access API");
  }
  return errors;
}

// 全屏唯一入口：状态查重 + 处理 requestFullscreen 的 Promise。
// Chrome 在无 transient user activation（约 5 秒窗口）时以
// TypeError "Permissions check failed" 拒绝；此时不报错，
// 记为待执行，等下一次真实点击/按键（手势）时重试。
let fullscreenDeferred = false;

function requestFullscreenWithGesture(canvas) {
  Promise.resolve(canvas.requestFullscreen()).catch((error) => {
    const message = error instanceof Error ? error.message : String(error);
    if (/Permissions check failed/.test(message)) {
      fullscreenDeferred = true;
      return;
    }
    console.warn("eden fullscreen failed:", error);
  });
}

for (const type of ["pointerdown", "keydown"]) {
  window.addEventListener(type, () => {
    if (!fullscreenDeferred) return;
    fullscreenDeferred = false;
    const canvas = document.getElementById("eden-canvas");
    if (canvas && !document.fullscreenElement) {
      requestFullscreenWithGesture(canvas);
    }
  }, { capture: true });
}

globalThis.edenApplyFullscreen = (canvas, enabled) => {
  const document = canvas.ownerDocument;
  if (enabled) {
    if (document.fullscreenElement === canvas) return;
    requestFullscreenWithGesture(canvas);
  } else {
    fullscreenDeferred = false;
    if (document.fullscreenElement == null) return;
    Promise.resolve(document.exitFullscreen()).catch((error) => {
      console.warn("eden exit fullscreen failed:", error);
    });
  }
};

window.addEventListener("unhandledrejection", (event) => {
  const reason = event.reason instanceof Error ? event.reason.message : String(event.reason);
  // requestFullscreen 无手势拒绝：功能已由 edenApplyFullscreen 接管，
  // 不作为页面错误展示。
  if (/Permissions check failed/.test(reason)) {
    event.preventDefault();
    return;
  }
  console.error("Unhandled promise rejection", event.reason);
  showError(`未处理的异步错误：${reason}`);
  event.preventDefault();
});

async function boot() {
  const errors = capabilityErrors();
  if (errors.length > 0) {
    showError(errors.join("；"));
    return;
  }

  try {
    const module = await import("/pkg/eden_web.js");
    await module.default({ module_or_path: "/pkg/eden_web_bg.wasm" });
  } catch (error) {
    console.error("eden WASM initialization failed", error);
    const detail = error instanceof Error ? error.message : String(error);
    showError(`WASM 加载失败：${detail}`);
  }
}

void boot();
