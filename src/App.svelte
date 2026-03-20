<script>
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
  import { open as openDialog, save } from '@tauri-apps/plugin-dialog';
  import { openPath } from '@tauri-apps/plugin-opener';

  const compressionOptions = [
    { value: 0, label: '0 - default (72dpi, 平衡)' },
    { value: 1, label: '1 - prepress (高质量, 300dpi)' },
    { value: 2, label: '2 - printer (高质量, 300dpi)' },
    { value: 3, label: '3 - ebook (中等质量, 150dpi)' },
    { value: 4, label: '4 - screen (低质量, 72dpi)' }
  ];

  let inputPath = '';
  let outputPath = '';
  let compressionLevel = 2;
  let isCompressing = false;
  let statusMessage = '请选择或拖拽一个 PDF 文件。';
  let errorMessage = '';
  let result = null;
  let progress = 0;
  let elapsedSeconds = 0;
  let progressTimer = null;
  let elapsedTimer = null;

  const formatBytes = (bytes) => {
    if (!Number.isFinite(bytes) || bytes < 0) return '-';
    if (bytes < 1024) return `${bytes} B`;
    const units = ['KB', 'MB', 'GB'];
    let value = bytes / 1024;
    let i = 0;
    while (value >= 1024 && i < units.length - 1) {
      value /= 1024;
      i += 1;
    }
    return `${value.toFixed(2)} ${units[i]}`;
  };

  const formatRatio = (ratio) => {
    if (!Number.isFinite(ratio)) return '-';
    if (ratio >= 0) {
      return `减少 ${(ratio * 100).toFixed(1)}%`;
    }
    return `增大 ${(Math.abs(ratio) * 100).toFixed(1)}%`;
  };

  async function setInputPath(path) {
    inputPath = path;
    outputPath = await deriveOutputPath(inputPath);
    result = null;
    errorMessage = '';
    statusMessage = '已选择输入文件。';
  }

  onMount(() => {
    let unlisten;
    getCurrentWebviewWindow()
      .onDragDropEvent((event) => {
        if (event.payload.type !== 'drop') return;

        const droppedPath = event.payload.paths?.[0];
        if (!droppedPath) return;
        if (!droppedPath.toLowerCase().endsWith('.pdf')) {
          errorMessage = '仅支持拖拽 PDF 文件。';
          return;
        }

        setInputPath(droppedPath);
        statusMessage = '已接收拖拽文件。';
      })
      .then((fn) => {
        unlisten = fn;
      })
      .catch(() => {
        // Ignore event registration error and keep dialog flow available.
      });

    return () => {
      stopLoadingState();
      if (unlisten) {
        unlisten();
      }
    };
  });

  const deriveOutputPath = async (inPath) => {
    try {
      return await invoke('suggest_output_path', { inputPath: inPath });
    } catch {
      return '';
    }
  };

  async function pickInput() {
    const selected = await openDialog({
      multiple: false,
      filters: [{ name: 'PDF', extensions: ['pdf'] }]
    });

    if (!selected || Array.isArray(selected)) return;

    await setInputPath(selected);
  }

  async function pickOutput() {
    const selected = await save({
      defaultPath: outputPath || 'compressed.pdf',
      filters: [{ name: 'PDF', extensions: ['pdf'] }]
    });

    if (!selected || Array.isArray(selected)) return;
    outputPath = selected;
  }

  async function compressPdf() {
    errorMessage = '';
    result = null;

    if (!inputPath) {
      errorMessage = '请先选择输入 PDF。';
      return;
    }
    if (!outputPath) {
      errorMessage = '请先选择输出路径。';
      return;
    }

    if (!outputPath.toLowerCase().endsWith('.pdf')) {
      outputPath = `${outputPath}.pdf`;
    }

    startLoadingState();
    isCompressing = true;
    statusMessage = '压缩中，请稍候...';

    try {
      const res = await invoke('compress_pdf', {
        req: {
          inputPath,
          outputPath,
          compressionLevel: Number(compressionLevel)
        }
      });
      result = res;
      statusMessage = '压缩完成。';
      finishLoadingState();
    } catch (err) {
      errorMessage = typeof err === 'string' ? err : JSON.stringify(err);
      statusMessage = '压缩失败。';
      stopLoadingState();
    } finally {
      isCompressing = false;
    }
  }

  function startLoadingState() {
    stopLoadingState();
    progress = 3;
    elapsedSeconds = 0;

    progressTimer = setInterval(() => {
      if (progress >= 92) return;
      const step = Math.max(1, Math.floor((95 - progress) / 8));
      progress = Math.min(92, progress + step);
    }, 450);

    elapsedTimer = setInterval(() => {
      elapsedSeconds += 1;
    }, 1000);
  }

  function finishLoadingState() {
    progress = 100;
    stopLoadingState(false);
  }

  function stopLoadingState(resetProgress = true) {
    if (progressTimer) {
      clearInterval(progressTimer);
      progressTimer = null;
    }
    if (elapsedTimer) {
      clearInterval(elapsedTimer);
      elapsedTimer = null;
    }
    if (resetProgress) {
      progress = 0;
      elapsedSeconds = 0;
    }
  }
</script>

<main class="container">
  <h1>PDF 压缩器（Tauri + Ghostscript）</h1>

  <section
    class="dropzone"
  >
    <p>拖拽 PDF 到窗口任意位置，或点击按钮选择文件</p>
    <button on:click={pickInput} disabled={isCompressing}>选择输入 PDF</button>
  </section>

  <section class="form-row">
    <label for="input">输入路径</label>
    <input id="input" type="text" bind:value={inputPath} readonly />
  </section>

  <section class="form-row">
    <label for="output">输出路径</label>
    <div class="inline">
      <input id="output" type="text" bind:value={outputPath} readonly />
      <button on:click={pickOutput} disabled={isCompressing}>选择输出</button>
    </div>
  </section>

  <section class="form-row">
    <label for="level">压缩档位</label>
    <select id="level" bind:value={compressionLevel} disabled={isCompressing}>
      {#each compressionOptions as option}
        <option value={option.value}>{option.label}</option>
      {/each}
    </select>
  </section>

  <section class="actions">
    <button class="primary" on:click={compressPdf} disabled={isCompressing}>
      {isCompressing ? '压缩中...' : '开始压缩'}
    </button>
    {#if result?.outputPath}
      <button on:click={() => openPath(result.outputPath)}>打开输出文件</button>
    {/if}
  </section>

  <section class="status">
    <p>{statusMessage}</p>
    {#if isCompressing}
      <div class="progress-wrap" aria-live="polite">
        <div class="spinner" aria-hidden="true"></div>
        <div class="progress-meta">
          <div class="progress-track">
            <div class="progress-bar" style={`width: ${progress}%`}></div>
          </div>
          <p class="progress-text">处理中... 已耗时 {elapsedSeconds}s</p>
        </div>
      </div>
    {/if}
    {#if errorMessage}
      <p class="error">错误：{errorMessage}</p>
    {/if}
  </section>

  {#if result}
    <section class="result">
      <h2>压缩结果</h2>
      <ul>
        <li>输入文件：{result.inputPath}</li>
        <li>输出文件：{result.outputPath}</li>
        <li>压缩前：{formatBytes(result.initialSizeBytes)}</li>
        <li>压缩后：{formatBytes(result.finalSizeBytes)}</li>
        <li>体积变化：{formatRatio(result.ratio)}</li>
      </ul>
    </section>
  {/if}
</main>
