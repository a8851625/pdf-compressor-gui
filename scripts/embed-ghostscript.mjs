import fs from 'node:fs/promises';
import path from 'node:path';
import process from 'node:process';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';

const execFileAsync = promisify(execFile);

const repoRoot = process.cwd();
const resourcesRoot = path.join(repoRoot, 'src-tauri', 'resources', 'ghostscript');

async function exists(filePath) {
  try {
    await fs.access(filePath);
    return true;
  } catch {
    return false;
  }
}

async function ensureCleanDir(dirPath) {
  await fs.rm(dirPath, { recursive: true, force: true });
  await fs.mkdir(dirPath, { recursive: true });
}

async function copyFileWithMode(source, target, mode = 0o755) {
  await fs.mkdir(path.dirname(target), { recursive: true });
  await fs.copyFile(source, target);
  await fs.chmod(target, mode);
}

async function copyDirRecursive(source, target) {
  await fs.mkdir(target, { recursive: true });
  const entries = await fs.readdir(source, { withFileTypes: true });
  for (const entry of entries) {
    const src = path.join(source, entry.name);
    const dst = path.join(target, entry.name);
    if (entry.isDirectory()) {
      await copyDirRecursive(src, dst);
      continue;
    }
    if (entry.isSymbolicLink()) {
      const real = await fs.realpath(src);
      const stat = await fs.stat(real);
      if (stat.isDirectory()) {
        await copyDirRecursive(real, dst);
      } else {
        await copyFileWithMode(real, dst, 0o755);
      }
      continue;
    }
    await copyFileWithMode(src, dst, 0o755);
  }
}

async function parseOtoolDependencies(binaryPath) {
  const { stdout } = await execFileAsync('otool', ['-L', binaryPath]);
  return stdout
    .split('\n')
    .slice(1)
    .map((line) => line.trim())
    .filter(Boolean)
    .map((line) => line.split(' (')[0])
    .filter((dep) => {
      if (dep.startsWith('/usr/lib/')) return false;
      if (dep.startsWith('/System/')) return false;
      return true;
    });
}

function isAbsoluteNonSystem(dep) {
  return dep.startsWith('/') && !dep.startsWith('/usr/lib/') && !dep.startsWith('/System/');
}

function dylibAliases(fileName) {
  if (!fileName.endsWith('.dylib')) {
    return [];
  }
  const parts = fileName.split('.');
  if (parts.length < 4) {
    return [];
  }

  const base = parts[0];
  const major = parts[1];
  const aliases = [`${base}.${major}.dylib`, `${base}.dylib`]
    .filter((name) => name !== fileName);
  return Array.from(new Set(aliases));
}

async function listDylibFiles(dirPath) {
  const files = await fs.readdir(dirPath, { withFileTypes: true });
  const output = [];
  for (const file of files) {
    if (file.isDirectory()) {
      continue;
    }
    const full = path.join(dirPath, file.name);
    if (file.isSymbolicLink()) {
      const real = await fs.realpath(full);
      if (real.endsWith('.dylib')) {
        output.push(real);
      }
      continue;
    }
    if (file.name.endsWith('.dylib')) {
      output.push(full);
    }
  }
  return Array.from(new Set(output));
}

async function bundleMacGhostscript() {
  const gsSource = await resolveMacGhostscriptPath();
  if (!(await exists(gsSource))) {
    throw new Error(
      `未找到 macOS Ghostscript: ${gsSource}。请先安装 Ghostscript，或设置 GS_MAC_PATH 指向 gs 可执行文件。`
    );
  }

  const macRoot = path.join(resourcesRoot, 'macos');
  const binDir = path.join(macRoot, 'bin');
  const libDir = path.join(macRoot, 'lib');
  await ensureCleanDir(macRoot);
  await fs.mkdir(binDir, { recursive: true });
  await fs.mkdir(libDir, { recursive: true });

  const gsTarget = path.join(binDir, 'gs');
  await copyFileWithMode(gsSource, gsTarget, 0o755);

  const toProcess = [gsSource];
  const copied = new Map();
  const depDirs = new Set();

  while (toProcess.length > 0) {
    const current = toProcess.pop();
    const deps = await parseOtoolDependencies(current);
    for (const dep of deps) {
      if (!isAbsoluteNonSystem(dep)) {
        continue;
      }
      const realDep = await fs.realpath(dep);
      depDirs.add(path.dirname(realDep));
      if (copied.has(realDep)) {
        continue;
      }
      const depName = path.basename(realDep);
      const target = path.join(libDir, depName);
      await copyFileWithMode(realDep, target, 0o755);
      copied.set(realDep, target);
      toProcess.push(realDep);
    }
  }

  for (const depDir of depDirs) {
    const dylibs = await listDylibFiles(depDir);
    for (const dylib of dylibs) {
      const realDylib = await fs.realpath(dylib);
      if (copied.has(realDylib)) {
        continue;
      }
      const dylibName = path.basename(realDylib);
      const target = path.join(libDir, dylibName);
      await copyFileWithMode(realDylib, target, 0o755);
      copied.set(realDylib, target);
    }
  }

  const libEntriesForId = await fs.readdir(libDir);
  for (const fileName of libEntriesForId) {
    if (!fileName.endsWith('.dylib')) {
      continue;
    }
    const targetPath = path.join(libDir, fileName);
    await execFileAsync('install_name_tool', ['-id', `@loader_path/${fileName}`, targetPath]);
  }

  const libEntries = await fs.readdir(libDir);
  for (const fileName of libEntries) {
    const sourceFile = path.join(libDir, fileName);
    const aliases = dylibAliases(fileName);
    for (const alias of aliases) {
      const aliasPath = path.join(libDir, alias);
      if (await exists(aliasPath)) {
        continue;
      }
      await fs.symlink(fileName, aliasPath);
    }
  }

  const availableLibNames = new Set(await fs.readdir(libDir));

  async function patchLinkedDependencies(targetPath, replacementBase) {
    const deps = await parseOtoolDependencies(targetPath);
    for (const dep of deps) {
      const depName = path.basename(dep);
      if (!availableLibNames.has(depName)) {
        continue;
      }
      await execFileAsync('install_name_tool', ['-change', dep, `${replacementBase}/${depName}`, targetPath]);
    }
  }

  const libEntriesForPatch = await fs.readdir(libDir);
  for (const fileName of libEntriesForPatch) {
    if (!fileName.endsWith('.dylib')) {
      continue;
    }
    const targetPath = path.join(libDir, fileName);
    await patchLinkedDependencies(targetPath, '@loader_path');
    await execFileAsync('codesign', ['--force', '--sign', '-', targetPath]);
  }

  await patchLinkedDependencies(gsTarget, '@loader_path/../lib');
  await execFileAsync('codesign', ['--force', '--sign', '-', gsTarget]);

  const sourceShareRoot = path.resolve(gsSource, '..', '..', 'share', 'ghostscript');
  if (await exists(sourceShareRoot)) {
    const targetShareRoot = path.join(macRoot, 'share', 'ghostscript');
    await copyDirRecursive(sourceShareRoot, targetShareRoot);
  }

  console.log(`✅ 已内置 Ghostscript (macOS): ${macRoot}`);
}

async function resolveMacGhostscriptPath() {
  if (process.env.GS_MAC_PATH) {
    return process.env.GS_MAC_PATH;
  }

  const candidates = ['/opt/homebrew/bin/gs', '/usr/local/bin/gs'];
  for (const candidate of candidates) {
    if (await exists(candidate)) {
      return candidate;
    }
  }

  try {
    const { stdout } = await execFileAsync('which', ['gs']);
    const found = stdout.trim();
    if (found) {
      return found;
    }
  } catch {
    // ignore
  }

  return '/opt/homebrew/bin/gs';
}

async function resolveWindowsGhostscriptDir() {
  if (process.env.GS_WIN_DIR) {
    return process.env.GS_WIN_DIR;
  }

  const bases = [process.env.ProgramFiles, process.env['ProgramFiles(x86)']].filter(Boolean);
  for (const base of bases) {
    const gsRoot = path.join(base, 'gs');
    if (!(await exists(gsRoot))) {
      continue;
    }

    const entries = await fs.readdir(gsRoot, { withFileTypes: true }).catch(() => []);
    const versions = entries
      .filter((entry) => entry.isDirectory() && /^gs\d+/i.test(entry.name))
      .map((entry) => entry.name)
      .sort((a, b) => b.localeCompare(a, undefined, { numeric: true }));

    for (const version of versions) {
      const candidate = path.join(gsRoot, version);
      const gsExe = path.join(candidate, 'bin', 'gswin64c.exe');
      if (await exists(gsExe)) {
        return candidate;
      }
    }
  }

  return null;
}

async function bundleWindowsGhostscript() {
  const gsWinDir = await resolveWindowsGhostscriptDir();
  if (!gsWinDir) {
    throw new Error('未找到 Windows Ghostscript。请安装 Ghostscript 或设置 GS_WIN_DIR。');
  }

  if (!(await exists(gsWinDir))) {
    throw new Error(`GS_WIN_DIR 不存在: ${gsWinDir}`);
  }

  const winRoot = path.join(resourcesRoot, 'windows');
  await ensureCleanDir(winRoot);
  await copyDirRecursive(gsWinDir, winRoot);
  console.log(`✅ 已内置 Ghostscript (Windows): ${winRoot}`);
}

async function main() {
  await fs.mkdir(resourcesRoot, { recursive: true });

  const platform = process.platform;
  if (platform === 'darwin') {
    await bundleMacGhostscript();
  } else if (platform === 'win32') {
    await bundleWindowsGhostscript();
  } else {
    console.log('ℹ️ 当前平台非 macOS/Windows，跳过内置 Ghostscript。');
  }
}

main().catch((err) => {
  console.error(`❌ 内置 Ghostscript 失败: ${err.message}`);
  process.exit(1);
});
