" SPDX-License-Identifier: MIT
" Copyright (c) 2026 Jarkko Sakkinen

vim9script

import autoload 'readseek/config.vim'

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

export def RunRaw(argv: list<string>, stdin: string, Callback: func)
  var stdout: list<string> = []
  var stderr: list<string> = []
  var completed = false
  var timeout_id = 0

  def Finish(result: dict<any>)
    if completed
      return
    endif
    completed = true
    if timeout_id > 0
      timer_stop(timeout_id)
    endif
    Callback(result)
  enddef

  def OnStdout(channel: channel, message: string)
    add(stdout, message)
  enddef

  def OnStderr(channel: channel, message: string)
    add(stderr, message)
  enddef

  def OnExit(job: job, status: number)
    var out = join(stdout, "\n")
    var err = join(stderr, "\n")
    var result: dict<any> = {ok: status == 0, status: status, stdout: out, stderr: err}
    if status != 0
      result.error = empty(err) ? $'readseek exited with status {status}' : err
    endif
    Finish(result)
  enddef

  var process = job_start([config.ExecutablePath()] + argv, {
    out_cb: OnStdout,
    err_cb: OnStderr,
    exit_cb: OnExit,
    out_mode: 'nl',
    err_mode: 'nl',
  })
  if job_status(process) == 'fail'
    Finish({
      ok: false,
      status: -1,
      stdout: '',
      stderr: '',
      error: $'failed to start readseek: {config.ExecutablePath()}',
    })
    return
  endif

  def OnTimeout(timer_id: number)
    if completed
      return
    endif
    job_stop(process)
    Finish({
      ok: false,
      status: -1,
      stdout: join(stdout, "\n"),
      stderr: join(stderr, "\n"),
      error: $'readseek timed out after {config.JobTimeout()}ms',
    })
  enddef

  timeout_id = timer_start(config.JobTimeout(), OnTimeout)

  var channel = job_getchannel(process)
  if !empty(stdin)
    ch_sendraw(channel, stdin)
  endif
  ch_close_in(channel)
enddef