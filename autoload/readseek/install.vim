" SPDX-License-Identifier: MIT
" Copyright (c) 2026 Jarkko Sakkinen

vim9script

import autoload 'readseek/config.vim'

const GithubRepo = 'jarkkojs/readseek'

export def Install(Callback: func, force: bool = false)
  if !empty(get(g:, 'readseek_executable', ''))
    Callback({ok: false, error: 'g:readseek_executable is externally managed'})
    return
  endif

  var platform = Platform()
  if empty(platform)
    Callback({ok: false, error: 'unsupported platform'})
    return
  endif

  var version = config.PluginVersion
  var dest = config.LocalBinaryPath()
  if !force && filereadable(dest) && config.VersionAt(dest) ==# version
    Callback({ok: true, path: dest, version: version, changed: false})
    return
  endif

  var dest_dir = config.LocalBinaryDir()
  if !isdirectory(dest_dir)
    mkdir(dest_dir, 'p')
  endif

  var asset = $'readseek-{version}-{platform}.tar.gz'
  var url = $'https://github.com/{GithubRepo}/releases/download/{version}/{asset}'
  var archive = tempname() .. '.tar.gz'
  Download(url, archive, (ok: bool, err: string) => {
    if !ok
      delete(archive)
      Callback({ok: false, error: $'download failed: {err}'})
      return
    endif

    var binary = has('win32') ? 'readseek.exe' : 'readseek'
    var stage_dir = $'{dest_dir}/.readseek-install-{getpid()}-{localtime()}'
    mkdir(stage_dir, 'p')
    var extract_out = system($'tar -xzf {shellescape(archive)} -C {shellescape(stage_dir)} {shellescape(binary)}')
    delete(archive)
    var staged = stage_dir .. '/' .. binary
    if v:shell_error != 0 || !filereadable(staged)
      delete(stage_dir, 'rf')
      Callback({ok: false, error: $'extraction failed: {trim(extract_out)}'})
      return
    endif

    if !has('win32')
      setfperm(staged, 'rwxr-xr-x')
    endif
    if config.VersionAt(staged) !=# version
      delete(stage_dir, 'rf')
      Callback({ok: false, error: 'staged binary does not match the plugin version'})
      return
    endif

    var backup = $'{dest}.previous-{getpid()}'
    var had_existing = filereadable(dest)
    if had_existing && rename(dest, backup) != 0
      delete(stage_dir, 'rf')
      Callback({ok: false, error: 'failed to preserve the existing readseek binary'})
      return
    endif
    if rename(staged, dest) != 0
      if had_existing
        rename(backup, dest)
      endif
      delete(stage_dir, 'rf')
      Callback({ok: false, error: 'failed to activate the staged readseek binary'})
      return
    endif

    delete(backup)
    delete(stage_dir, 'rf')
    config.InvalidateHealthCache()
    Callback({ok: true, path: dest, version: version, changed: true})
  })
enddef

export def Uninstall(Callback: func)
  if !empty(get(g:, 'readseek_executable', ''))
    Callback({ok: false, error: 'g:readseek_executable is externally managed'})
    return
  endif

  var dest = config.LocalBinaryPath()
  if !filereadable(dest)
    Callback({ok: true, changed: false})
    return
  endif
  if delete(dest) != 0
    Callback({ok: false, error: $'failed to remove {dest}'})
    return
  endif
  config.InvalidateHealthCache()
  Callback({ok: true, changed: true})
enddef

export def Platform(): string
  if has('win32')
    return 'win32-x64'
  endif

  var arch = trim(system('uname -m'))
  if has('mac') && (arch ==# 'arm64' || arch ==# 'aarch64')
    return 'darwin-arm64'
  elseif has('unix') && (arch ==# 'arm64' || arch ==# 'aarch64')
    return 'linux-arm64'
  elseif has('unix') && (arch ==# 'x86_64' || arch ==# 'amd64')
    return 'linux-x64'
  endif
  return ''
enddef

def Download(url: string, dest: string, Callback: func)
  var cmd: list<string>
  if executable('curl')
    cmd = ['curl', '-fsSL', '-o', dest, url]
  elseif executable('wget')
    cmd = ['wget', '-q', '-O', dest, url]
  else
    Callback(false, 'curl or wget required for installation')
    return
  endif

  var stderr_lines: list<string> = []
  def OnStderr(channel: channel, message: string)
    add(stderr_lines, message)
  enddef
  def OnExit(job_obj: job, status: number)
    Callback(status == 0, join(stderr_lines, "\n"))
  enddef
  job_start(cmd, {err_cb: OnStderr, exit_cb: OnExit, err_mode: 'nl'})
enddef