" SPDX-License-Identifier: MIT
" Copyright (c) 2026 Jarkko Sakkinen

vim9script

import autoload 'readseek/config.vim'

const GithubRepo = 'jarkkojs/readseek'

export def Install(Callback: func)
  var platform = Platform()
  if empty(platform)
    Callback({ok: false, error: 'unsupported platform'})
    return
  endif

  var version = config.PluginVersion
  var asset = $'readseek-{version}-{platform}.tar.gz'
  var url = $'https://github.com/{GithubRepo}/releases/download/{version}/{asset}'
  var dest_dir = config.LocalBinaryDir()

  if !isdirectory(dest_dir)
    mkdir(dest_dir, 'p')
  endif

  var tmpfile = tempname() .. '.tar.gz'

  Download(url, tmpfile, (ok: bool, err: string) => {
    if !ok
      delete(tmpfile)
      Callback({ok: false, error: $'download failed: {err}'})
      return
    endif

    var binary_name = has('win32') ? 'readseek.exe' : 'readseek'
    var dest = config.LocalBinaryPath()
    var tmpdir = $'{dest_dir}/.install-{getpid()}-{localtime()}'
    mkdir(tmpdir, 'p')
    var extract_out = system($'tar -xzf {shellescape(tmpfile)} -C {shellescape(tmpdir)} {shellescape(binary_name)}')
    delete(tmpfile)
    var staged = tmpdir .. '/' .. binary_name

    if v:shell_error != 0 || !filereadable(staged)
      delete(tmpdir, 'rf')
      Callback({ok: false, error: $'extraction failed: {trim(extract_out)}'})
      return
    endif

    if !has('win32')
      setfperm(staged, 'rwxr-xr-x')
    endif
    delete(dest)
    if rename(staged, dest) != 0
      delete(tmpdir, 'rf')
      Callback({ok: false, error: 'failed to install release binary'})
      return
    endif
    delete(tmpdir, 'rf')

    config.InvalidateHealthCache()
    Callback({ok: true, path: dest, version: version})
  })
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

  job_start(cmd, {
    err_cb: OnStderr,
    exit_cb: OnExit,
    err_mode: 'nl',
  })
enddef
