" SPDX-License-Identifier: MIT
" Copyright (c) 2026 Jarkko Sakkinen

vim9script

import autoload 'readseek/buffer.vim'
import autoload 'readseek/config.vim'
import autoload 'readseek/job.vim'
import autoload 'readseek/quickfix.vim'
import autoload 'readseek/root.vim'

export def CheckHealth()
  config.CheckHealth()

  var exec_ok = config.IsExecutableAvailable()
  var exec_path = config.ExecutablePath()
  var version = exec_ok ? config.Version() : ''
  var version_ok = exec_ok && version ==# config.PluginVersion
  var project_root = root.Find()

  var rows: list<dict<any>> = [
    StatusRow(exec_ok, 'executable', exec_ok ? exec_path : $'{exec_path} (not installed)'),
    StatusRow(version_ok, 'version', empty(version) ? $'expected {config.PluginVersion}' : $'readseek {version} (expected {config.PluginVersion})'),
    {marker: '•', highlight: 'ReadSeekInfo', label: 'root', value: project_root},
  ]

  var plain: list<string> = []
  for row in rows
    add(plain, $'{row.marker} {row.label}: {row.value}')
  endfor

  if !exists('*popup_create')
    for line in plain
      echo line
    endfor
    return
  endif

  EnsurePropTypes()
  var items: list<dict<any>> = []
  for row in rows
    var text = $'{row.marker} {row.label}: {row.value}'
    add(items, {
      text: text,
      props: [
        {col: 1, length: strlen(row.marker), type: PropFor(row.highlight)},
        {col: strlen(row.marker) + 2, length: strlen(row.label), type: 'ReadSeekPropTitle'},
      ],
    })
  endfor

  Popup(items, {
    pos: 'center',
    title: ' readseek health ',
    close: 'click',
  })
enddef

def StatusRow(ok: bool, label: string, value: string): dict<any>
  return {
    marker: ok ? '✓' : '✗',
    highlight: ok ? 'ReadSeekOk' : 'ReadSeekError',
    label: label,
    value: value,
  }
enddef

export def Hover()
  Identify((result: dict<any>) => {
    if !result.ok
      Notify(get(result, 'error', 'readseek identify failed'), 'error')
      return
    endif

    var lines = HoverLines(result.json)
    if empty(lines)
      Notify('no identifier at cursor', 'info')
      return
    endif

    ShowHover(lines)
  })
enddef

export def Definition()
  var project_root = root.Find()
  Status('finding definition...')
  Identify((identify_result: dict<any>) => {
    if !identify_result.ok
      Notify(get(identify_result, 'error', 'readseek identify failed'), 'error')
      return
    endif

    var identifier_text = IdentifierText(identify_result.json)
    if empty(identifier_text)
      Notify('no identifier at cursor', 'info')
      return
    endif

    job.Run(['def', project_root, identifier_text, '--format', 'plain'], '', (definition_result: dict<any>) => {
      if !definition_result.ok
        Notify(get(definition_result, 'error', 'readseek definition failed'), 'error')
        return
      endif
      HandleDefinitionLocations(get(definition_result.json, 'locations', []), project_root)
    })
  })
enddef

export def References()
  Identify((identify_result: dict<any>) => {
    if !identify_result.ok
      Notify(get(identify_result, 'error', 'readseek identify failed'), 'error')
      return
    endif

    var identifier_text = IdentifierText(identify_result.json)
    if empty(identifier_text)
      Notify('no identifier at cursor', 'info')
      return
    endif

    var project_root = root.Find()
    Status($'finding references for {identifier_text}...')
    job.Run(['refs', '--format', 'plain', project_root, identifier_text], '', (references_result: dict<any>) => {
      if !references_result.ok
        Notify(get(references_result, 'error', 'readseek references failed'), 'error')
        return
      endif

      var locations = get(references_result.json, 'locations', [])
      if empty(locations)
        Notify($'no references found for {identifier_text}', 'info')
        return
      endif

      Status($'{len(locations)} {Plural(len(locations), 'reference')} found for {identifier_text}')
      for location in locations
        location.file = ResolveLocationFile(get(location, 'file', ''), project_root)
      endfor
      quickfix.SetLocations(locations, $'readseek references: {identifier_text}')
    })
  })
enddef

export def Rename()
  var file = expand('%:p')
  if empty(file) || !filereadable(file)
    Notify('rename requires the buffer to be a saved file', 'error')
    return
  endif
  if &modified
    Notify('save the buffer before renaming', 'error')
    return
  endif

  var line = buffer.Line()
  var column = buffer.ByteColumn()
  Identify((identify_result: dict<any>) => {
    if !identify_result.ok
      Notify(get(identify_result, 'error', 'readseek identify failed'), 'error')
      return
    endif

    var old_name = IdentifierText(identify_result.json)
    if empty(old_name)
      Notify('no identifier at cursor', 'info')
      return
    endif

    var new_name = input($'Rename {old_name} to: ', old_name)
    if empty(new_name) || new_name ==# old_name
      return
    endif

    var project_root = root.Find()
    RenameTo(file, line, column, old_name, new_name, project_root)
  })
enddef

# Apply a binding-accurate rename to file via readseek rename --apply.
export def RenameTo(file: string, line: number, column: number, old_name: string, new_name: string, workspace: string = '')
  Status($'renaming {old_name} to {new_name}...')
  var argv = ['rename', file,
    '--line', string(line),
    '--column', string(column),
    '--to', new_name, '--apply']
  if !empty(workspace)
    extend(argv, ['--workspace', workspace])
  endif
  job.Run(argv, '', (rename_result: dict<any>) => {
    if !rename_result.ok
      Notify(get(rename_result, 'error', 'readseek rename failed'), 'error')
      return
    endif

    var conflicts = get(rename_result.json, 'conflicts', [])
    if !empty(conflicts)
      Notify($'rename has {len(conflicts)} {Plural(len(conflicts), "conflict")}; not applied', 'warn')
      return
    endif

    var edits = get(rename_result.json, 'edits', [])
    var changed = {[file]: true}
    var dir = fnamemodify(file, ':h')
    for entry in get(rename_result.json, 'others', [])
      var path = get(entry, 'file', '')
      if !empty(path)
        changed[fnamemodify(path =~# '^/' ? path : dir .. '/' .. path, ':p')] = true
      endif
    endfor
    ReloadChangedBuffers(changed)
    Notify($'renamed {old_name} to {new_name} in {len(edits)} {Plural(len(edits), "location")}', 'ok')
  })
enddef

export def Search()
  var pattern = input('readseek pattern: ')
  if empty(pattern)
    return
  endif

  var project_root = root.Find()
  Status($'searching for {pattern}...')

  job.Run(['search', project_root, pattern], '', (result: dict<any>) => {
    if !result.ok
      var msg = get(result, 'stderr', 'search failed')
      Notify(empty(msg) ? 'search failed' : msg, 'error')
      return
    endif

    var locations = SearchLocations(result.json, project_root)
    if empty(locations)
      Notify($'no matches for {pattern}', 'info')
      return
    endif

    Notify($'{len(locations)} {Plural(len(locations), "match")} for {pattern}', 'ok')
    quickfix.SetLocations(locations, $'readseek search: {pattern}')
  })
enddef

export def Map()
  var path = buffer.Path()
  var tail = fnamemodify(path, ':t')
  Status('mapping ' .. tail .. '...')

  job.Run(['map', $'stdin:{path}'], buffer.Stdin(), (result: dict<any>) => {
    if !result.ok
      Notify(get(result, 'error', 'readseek map failed'), 'error')
      return
    endif

    var symbols = get(result.json, 'symbols', [])
    if empty(symbols)
      Notify('no symbols found', 'info')
      return
    endif

    var locations: list<any> = []
    for symbol in symbols
      var kind = get(symbol, 'kind', 'symbol')
      var name = get(symbol, 'name', '')
      var entry = {
        file: path,
        line: get(symbol, 'start_line', 1),
        column: 1,
        text: kind .. ' ' .. name,
      }
      add(locations, entry)
    endfor

    var count = len(locations)
    Status(count .. ' ' .. Plural(count, 'symbol') .. ' found')
    quickfix.SetLocations(locations, 'readseek map: ' .. tail)
  })
enddef

export def Check()
  var path = buffer.Path()
  Status($'checking {fnamemodify(path, ":t")}...')
  job.Run(['check', $'stdin:{path}'], buffer.Stdin(), (result: dict<any>) => {
    if !result.ok
      Notify(get(result, 'error', 'readseek check failed'), 'error')
      return
    endif
    var locations = DiagnosticsLocations(result.json, path)
    if empty(locations)
      Notify('no parser diagnostics', 'ok')
      return
    endif
    quickfix.SetLocations(locations, $'readseek check: {fnamemodify(path, ":t")}')
    Notify($'{len(locations)} {Plural(len(locations), "diagnostic")} found', 'warn')
  })
enddef

export def DiagnosticsLocations(json: dict<any>, path: string): list<any>
  var locations: list<any> = []
  for diagnostic in get(json, 'diagnostics', [])
    var kind = get(diagnostic, 'kind', 'error')
    var start_line = get(diagnostic, 'start_line', 1)
    add(locations, {
      file: path,
      line: start_line,
      column: 1,
      text: $'[{kind}] parser diagnostic',
    })
  endfor
  return locations
enddef

export def Init()
  var project_root = root.Find()
  Status($'initializing cache in {project_root}...')

  job.RunRaw(['init', project_root], '', (result: dict<any>) => {
    if !result.ok
      Notify(get(result, 'error', 'readseek init failed'), 'error')
      return
    endif

    var message = trim(result.stdout)
    Notify(empty(message) ? $'cache initialized in {project_root}' : message, 'ok')
  })
enddef

export def InstallComplete(result: dict<any>)
  if !get(result, 'ok', false)
    Notify(get(result, 'error', 'readseek install failed'), 'error')
    return
  endif
  if !get(result, 'changed', false)
    Notify('readseek is already installed', 'info')
    return
  endif
  if has_key(result, 'path')
    Notify($'readseek {get(result, "version", "")} installed at {result.path}', 'ok')
    return
  endif
  Notify('readseek removed', 'ok')
enddef

export def Identify(Callback: func)
  job.Run(buffer.IdentifyArgs(), buffer.Stdin(), Callback)
enddef

def IdentifierText(identify: dict<any>): string
  var identifier = get(identify, 'identifier', v:null)
  if type(identifier) != v:t_dict || !has_key(identifier, 'text')
    return ''
  endif
  return identifier.text
enddef

def HandleDefinitionLocations(locations: list<any>, project_root: string)
  if empty(locations)
    Notify('no definitions found', 'info')
    return
  endif

  if len(locations) == 1
    Status('1 definition found')
    OpenLocation(locations[0], project_root)
    return
  endif

  Status($'{len(locations)} definitions found')
  for location in locations
    location.file = ResolveLocationFile(get(location, 'file', ''), project_root)
  endfor
  quickfix.SetLocations(locations, 'readseek definitions')
enddef

def OpenLocation(location: dict<any>, project_root: string)
  var file = ResolveLocationFile(get(location, 'file', ''), project_root)
  if empty(file)
    Notify('definition result has no file', 'error')
    return
  endif

  execute 'edit ' .. fnameescape(file)
  cursor(get(location, 'line', 1), get(location, 'column', 1))
enddef

def ReloadChangedBuffers(changed: dict<bool>)
  var save_autoread = &autoread
  set autoread
  try
    for file in keys(changed)
      var buffer_number = bufnr(file)
      if buffer_number == -1
        continue
      endif
      var views: list<dict<any>> = []
      if exists('*win_findbuf') && exists('*win_execute')
        for window_id in win_findbuf(buffer_number)
          win_execute(window_id, 'legacy let g:readseek_saved_view = winsaveview()')
          add(views, {id: window_id, view: g:readseek_saved_view})
        endfor
        unlet! g:readseek_saved_view
      endif
      execute $'checktime {buffer_number}'
      for entry in views
        var view_str = string(entry.view)
        win_execute(entry.id, 'legacy call winrestview(' .. view_str .. ')')
      endfor
    endfor
  finally
    &autoread = save_autoread
  endtry
enddef

export def SearchLocations(json: dict<any>, project_root: string): list<any>
  var locations: list<any> = []
  for file_entry in get(json, 'results', [])
    var file = ResolveLocationFile(get(file_entry, 'file', ''), project_root)
    for match_entry in get(file_entry, 'matches', [])
      var hashlines = get(match_entry, 'hashlines', [])
      var text = empty(hashlines) ? '' : get(hashlines[0], 'text', '')
      add(locations, {
        file: file,
        line: get(match_entry, 'start_line', 1),
        column: 1,
        text: text,
      })
    endfor
  endfor
  return locations
enddef

def ResolveLocationFile(file: string, project_root: string): string
  if empty(file)
    return ''
  endif

  if file =~# '^/'
    return fnamemodify(file, ':p')
  endif

  return fnamemodify(project_root .. '/' .. substitute(file, '^\./', '', ''), ':p')
enddef

export def HoverLines(identify: dict<any>): list<string>
  var lines: list<string> = []
  var identifier = get(identify, 'identifier', v:null)
  if type(identifier) == v:t_dict && has_key(identifier, 'text')
    add(lines, $'identifier: {identifier.text}')
  endif

  var symbol = get(identify, 'symbol', v:null)
  if type(symbol) == v:t_dict && has_key(symbol, 'name')
    add(lines, $'symbol: {symbol.name}')
    if has_key(symbol, 'kind')
      add(lines, $'kind: {symbol.kind}')
    endif
  endif

  if has_key(identify, 'file') && has_key(identify, 'line') && has_key(identify, 'column')
    add(lines, $'location: {identify.file}:{identify.line}:{identify.column}')
  endif

  return lines
enddef

def ShowHover(lines: list<string>)
  if !exists('*popup_create')
    echo join(lines, ' | ')
    return
  endif

  var title = empty(lines) ? ' readseek ' : $' {lines[0]} '
  Popup(lines, {
    pos: 'botleft',
    line: 'cursor+1',
    col: 'cursor',
    padding: [0, 1, 0, 1],
    title: title,
    scrollbar: 1,
  })
enddef

# Shared popup styling: rounded border, readseek highlights, dismiss on move.
def Popup(content: any, overrides: dict<any>)
  var options: dict<any> = {
    padding: [1, 2, 1, 2],
    border: [1, 1, 1, 1],
    borderchars: ['─', '│', '─', '│', '╭', '╮', '╯', '╰'],
    borderhighlight: ['ReadSeekBorder'],
    highlight: 'ReadSeekFloat',
    title: ' readseek ',
    scrollbar: 0,
    wrap: false,
    moved: 'any',
    zindex: 300,
  }
  popup_create(content, extend(options, overrides))
enddef

const PropTypes = {
  ReadSeekOk: 'ReadSeekPropOk',
  ReadSeekError: 'ReadSeekPropError',
  ReadSeekWarn: 'ReadSeekPropWarn',
  ReadSeekInfo: 'ReadSeekPropInfo',
}

def PropFor(highlight: string): string
  return get(PropTypes, highlight, 'ReadSeekPropInfo')
enddef

def EnsurePropTypes()
  if !exists('*prop_type_add')
    return
  endif
  for [highlight, prop] in items(PropTypes)
    if empty(prop_type_get(prop))
      prop_type_add(prop, {highlight: highlight})
    endif
  endfor
  if empty(prop_type_get('ReadSeekPropTitle'))
    prop_type_add('ReadSeekPropTitle', {highlight: 'ReadSeekTitle'})
  endif
enddef

def Notify(message: string, level: string = 'info')
  if !exists('*popup_notification')
    if level == 'error'
      echohl ErrorMsg
    elseif level == 'warn'
      echohl WarningMsg
    endif
    echomsg $'readseek.vim: {message}'
    echohl None
    return
  endif

  var highlight = 'ReadSeekInfo'
  if level == 'error'
    highlight = 'ReadSeekError'
  elseif level == 'warn'
    highlight = 'ReadSeekWarn'
  elseif level == 'ok'
    highlight = 'ReadSeekOk'
  endif

  popup_notification($' readseek.vim: {message} ', {
    highlight: highlight,
    time: config.NotificationTimeout(),
    pos: 'topright',
    line: 1,
    col: 1,
  })
enddef

def Status(message: string)
  echohl ModeMsg
  echomsg 'readseek.vim: ' .. message
  echohl None
enddef

def StatusWarn(message: string)
  echohl WarningMsg
  echomsg 'readseek.vim: ' .. message
  echohl None
enddef

def Plural(count: number, word: string): string
  return count == 1 ? word : word .. 's'
enddef
