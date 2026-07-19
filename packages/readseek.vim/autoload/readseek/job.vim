" SPDX-License-Identifier: MIT
" Copyright (c) 2026 Jarkko Sakkinen

vim9script

import autoload 'readseek/config.vim'

# Run readseek and decode its JSON output before invoking Callback.
export def Run(argv: list<string>, stdin: string, Callback: func)
  RunRaw(argv, stdin, (result: dict<any>) => {
    if !result.ok
      Callback(result)
      return
    endif

    try
      result.json = json_decode(result.stdout)
    catch
      result.ok = false
      result.error = $'failed to decode readseek JSON output: {v:exception}'
    endtry

    Callback(result)
  })
enddef

# Run readseek and hand the raw stdout/stderr to Callback without decoding.
export def RunRaw(argv: list<string>, stdin: string, Callback: func)
  var stdout: list<string> = []
  var stderr: list<string> = []

  def OnStdout(channel: channel, message: string)
    add(stdout, message)
  enddef

  def OnStderr(channel: channel, message: string)
    add(stderr, message)
  enddef

  def OnExit(job: job, status: number)
    var out = join(stdout, "\n")
    var err = join(stderr, "\n")
    var result: dict<any> = {
      ok: status == 0,
      status: status,
      stdout: out,
      stderr: err,
    }

    if status != 0
      result.error = empty(err) ? $'readseek exited with status {status}' : err
    endif

    Callback(result)
  enddef

  var command = [config.ExecutablePath()] + argv
  var job = job_start(command, {
    out_cb: OnStdout,
    err_cb: OnStderr,
    exit_cb: OnExit,
    out_mode: 'nl',
    err_mode: 'nl',
  })

  if job_status(job) == 'fail'
    Callback({
      ok: false,
      status: -1,
      stdout: '',
      stderr: '',
      error: $'failed to start readseek: {config.ExecutablePath()}',
    })
    return
  endif

  var channel = job_getchannel(job)
  if !empty(stdin)
    ch_sendraw(channel, stdin)
  endif
  ch_close_in(channel)
enddef
