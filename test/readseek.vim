" SPDX-License-Identifier: MIT
" Copyright (c) 2026 Jarkko Sakkinen

vim9script
nnoremap <Plug>(ReadSeekHover) :echo 'keep'<CR>

set nomore
set rtp^=.
runtime plugin/readseek.vim
g:readseek_executable = ''
delete('test-readseek-failures.log')
delete('test-readseek.log')

var failures: list<string> = []

def Check(name: string, condition: bool)
  if !condition
    add(failures, name)
  endif
enddef

def TestQuickfixItems()
  var locations = [{
    file: 'autoload/readseek.vim',
    line: 3,
    column: 5,
    text: 'import autoload',
  }]
  var items = readseek#quickfix#ToItems(locations)
  Check('quickfix item count', len(items) == 1)
  Check('quickfix filename', items[0].filename ==# 'autoload/readseek.vim')
  Check('quickfix line', items[0].lnum == 3)
  Check('quickfix column', items[0].col == 5)
  Check('quickfix text', items[0].text ==# 'import autoload')

  locations = [{
    file: 'autoload/readseek.vim',
    line: 12,
    column: 1,
    text: 'export def CheckHealth()',
    kind: 'function',
    name: 'CheckHealth',
  }]
  items = readseek#quickfix#ToItems(locations)
  Check('quickfix metadata text', items[0].text ==# '[function] CheckHealth: export def CheckHealth()')
enddef

def TestResultLists()
  var locations = [{file: 'README.md', line: 1, column: 1, text: '# readseek.vim'}]
  edit README.md
  var source_buffer = bufnr('%')

  g:readseek_list_type = 'quickfix'
  readseek#quickfix#SetLocations(locations, 'quickfix test')
  Check('quickfix populated', len(getqflist()) == 1)
  Check('quickfix preserves source buffer', bufnr('%') == source_buffer)
  cclose

  g:readseek_list_type = 'location'
  readseek#quickfix#SetLocations(locations, 'location test')
  Check('location list populated', len(getloclist(0)) == 1)
  Check('location list preserves source buffer', bufnr('%') == source_buffer)
  lclose

  g:readseek_list_type = 'quickfix'
enddef

def TestMappings()
  var gd = maparg('<Plug>(ReadSeekDefinition)', 'n', false, true)
  var gr = maparg('<Plug>(ReadSeekReferences)', 'n', false, true)
  var hover = maparg('<Plug>(ReadSeekHover)', 'n', false, true)
  var rn = maparg('<Plug>(ReadSeekRename)', 'n', false, true)
  var search = maparg('<Plug>(ReadSeekSearch)', 'n', false, true)
  var map_sym = maparg('<Plug>(ReadSeekMap)', 'n', false, true)
  Check('definition plug mapping', !empty(gd) && gd.rhs ==# '<ScriptCmd>ReadSeekDefinition<CR>')
  Check('references plug mapping', !empty(gr) && gr.rhs ==# '<ScriptCmd>ReadSeekReferences<CR>')
  Check('hover plug mapping preserved', !empty(hover) && hover.rhs ==# ":echo 'keep'<CR>")
  Check('rename plug mapping', !empty(rn) && rn.rhs ==# '<ScriptCmd>ReadSeekRename<CR>')
  Check('search plug mapping', !empty(search) && search.rhs ==# '<ScriptCmd>ReadSeekSearch<CR>')
  Check('map plug mapping', !empty(map_sym) && map_sym.rhs ==# '<ScriptCmd>ReadSeekMap<CR>')
enddef

def TestIdentifyArgs()
  var base = tempname()
  mkdir(base .. '/project', 'p')
  writefile(['alpha beta'], base .. '/project/file.c')
  execute 'edit ' .. fnameescape(base .. '/project/file.c')
  cursor(1, 7)

  var args = readseek#buffer#IdentifyArgs()
  Check('identify command', args[0] ==# 'identify')
  Check('identify stdin target', args[1] ==# $'stdin:{expand('%:p')}:1')
  Check('identify column option', index(args, '--column') >= 0 && args[index(args, '--column') + 1] ==# '7')

  bwipe!
  delete(base, 'rf')
enddef

def TestRootMarkers()
  var base = tempname()
  mkdir(base .. '/project/src', 'p')
  writefile(['marker'], base .. '/project/.readseek-root')
  writefile(['vim9script'], base .. '/project/src/file.vim')

  g:readseek_root_markers = ['.readseek-root']
  execute 'edit ' .. fnameescape(base .. '/project/src/file.vim')
  Check('custom root marker', readseek#root#Find() ==# base .. '/project')

  g:readseek_root_markers = ['.git']
  delete(base, 'rf')
enddef

def TestHealthCache()
  var save_executable = g:readseek_executable
  g:readseek_executable = 'readseek-cache-a'
  readseek#config#CacheHealth('1.2.3')
  Check('health cache matches executable', readseek#config#IsHealthCached())

  g:readseek_executable = 'readseek-cache-b'
  Check('health cache tracks executable', !readseek#config#IsHealthCached())

  unlet! g:readseek_health
  g:readseek_executable = save_executable
enddef

def WaitForMessage(text: string): bool
  for _ in range(20)
    if execute('messages') =~# text
      return true
    endif
    sleep 50m
  endfor
  return false
enddef

def TestReferenceFeedback()
  var base = tempname()
  mkdir(base .. '/project/src', 'p')
  writefile(['marker'], base .. '/project/.git')
  writefile(['void target(void) { target(); }'], base .. '/project/src/file.c')

  var executable = base .. '/readseek-fake'
  writefile([
    '#!/bin/sh',
    'if [ "$1" = "identify" ]; then',
    '  printf ''{"identifier":{"text":"target"}}\n''',
    'elif [ "$1" = "refs" ]; then',
    '  printf ''{"locations":[{"file":"src/file.c","line":1,"column":6,"text":"void target(void) { target(); }"},{"file":"src/file.c","line":1,"column":21,"text":"void target(void) { target(); }"}]}\n''',
    '  [ "$2" = "--format" ] && [ "$3" = "plain" ] || exit 2',
    'else',
    '  printf ''{}\n''',
    'fi',
  ], executable)
  setfperm(executable, 'rwx------')

  var save_executable = g:readseek_executable
  var save_root_markers = g:readseek_root_markers
  g:readseek_executable = executable
  g:readseek_root_markers = ['.git']

  execute 'edit ' .. fnameescape(base .. '/project/src/file.c')
  cursor(1, 8)
  readseek#References()

  Check('references start feedback', WaitForMessage('readseek.vim: finding references for target...'))
  Check('references completion feedback', WaitForMessage('readseek.vim: 2 references found for target'))

  g:readseek_executable = save_executable
  g:readseek_root_markers = save_root_markers
  delete(base, 'rf')
enddef

def WaitFor(Cond: func(): bool): bool
  for _ in range(40)
    if Cond()
      return true
    endif
    sleep 25m
  endfor
  return false
enddef

def TestRename()
  var base = tempname()
  mkdir(base, 'p')
  var file = base .. '/file.c'
  writefile(['int target = target;'], file)

  # The fake applies the rename like readseek rename --apply: it rewrites the
  # file on disk and reports the plan as JSON on stdout.
  var executable = base .. '/readseek-fake'
  writefile([
    '#!/bin/sh',
    'if [ "$1" = "rename" ]; then',
    '  sed -i.bak ''s/target/renamed/g'' "$2"',
    '  printf ''{"old_name":"target","new_name":"renamed","applied":true,"conflicts":[],"edits":[{"line":1,"start_column":5},{"line":1,"start_column":14}]}\n''',
    'else',
    '  printf ''{}\n''',
    'fi',
  ], executable)
  setfperm(executable, 'rwx------')

  var save_executable = g:readseek_executable
  g:readseek_executable = executable

  execute 'edit ' .. fnameescape(file)
  readseek#RenameTo(file, 1, 5, 'target', 'renamed')

  var renamed = WaitFor((): bool => getline(1) ==# 'int renamed = renamed;')
  Check('rename rewrites and reloads buffer', renamed)

  g:readseek_executable = save_executable
  bwipe!
  delete(base, 'rf')
enddef

def TestRenameConflict()
  var base = tempname()
  mkdir(base, 'p')
  var file = base .. '/file.c'
  writefile(['int target = 0;'], file)

  # A conflict plan must not touch the file. The fake reports a conflict and
  # leaves the file untouched, mirroring readseek refusing to apply.
  var executable = base .. '/readseek-fake'
  writefile([
    '#!/bin/sh',
    'printf ''{"old_name":"target","new_name":"renamed","applied":false,"conflicts":[{"line":1,"column":5,"reason":"already bound"}],"edits":[]}\n''',
  ], executable)
  setfperm(executable, 'rwx------')

  var save_executable = g:readseek_executable
  g:readseek_executable = executable

  execute 'edit ' .. fnameescape(file)
  readseek#RenameTo(file, 1, 5, 'target', 'renamed')

  # Give the async job time to finish; the buffer must remain unchanged.
  sleep 300m
  Check('rename conflict leaves buffer unchanged', getline(1) ==# 'int target = 0;')

  g:readseek_executable = save_executable
  bwipe!
  delete(base, 'rf')
enddef

def TestRenameUnsupported()
  var base = tempname()
  mkdir(base, 'p')
  var file = base .. '/file.vim'
  writefile(['var target = 0'], file)

  # When readseek cannot rename (e.g. unsupported language), it exits non-zero
  # with a message on stderr. The plugin must leave the buffer unchanged.
  var executable = base .. '/readseek-fake'
  writefile([
    '#!/bin/sh',
    'printf ''error: rename not supported for vimscript\n'' >&2',
    'exit 1',
  ], executable)
  setfperm(executable, 'rwx------')

  var save_executable = g:readseek_executable
  g:readseek_executable = executable

  execute 'edit ' .. fnameescape(file)
  readseek#RenameTo(file, 1, 5, 'target', 'renamed')

  sleep 300m
  Check('rename unsupported leaves buffer unchanged', getline(1) ==# 'var target = 0')

  g:readseek_executable = save_executable
  bwipe!
  delete(base, 'rf')
enddef

def TestRenameRequiresSavedBuffer()
  var base = tempname()
  mkdir(base, 'p')
  # A sentinel-touching fake proves the executable is never invoked when the
  # buffer is unsaved: Rename() must bail before spawning any job.
  var executable = base .. '/readseek-fake'
  writefile(['#!/bin/sh', 'touch "' .. base .. '/invoked"', 'printf ''{}\n'''], executable)
  setfperm(executable, 'rwx------')

  var save_executable = g:readseek_executable
  g:readseek_executable = executable

  enew
  setline(1, 'int target = 0;')
  readseek#Rename()
  sleep 300m
  Check('rename refuses unsaved buffer', !filereadable(base .. '/invoked'))

  g:readseek_executable = save_executable
  bwipe!
  delete(base, 'rf')
enddef

def TestDefinitionUsesIdentifier()
  var base = tempname()
  mkdir(base .. '/project/src', 'p')
  writefile(['marker'], base .. '/project/.git')
  writefile(['void target(void) {}'], base .. '/project/src/file.c')

  var executable = base .. '/readseek-fake'
  writefile([
    '#!/bin/sh',
    'if [ "$1" = "identify" ]; then',
    '  printf ''{"identifier":{"text":"target"}}\n''',
    'elif [ "$1" = "def" ]; then',
    '  [ "$2" = "' .. base .. '/project" ] && [ "$3" = "target" ] && [ "$4" = "--format" ] && [ "$5" = "plain" ] || exit 2',
    '  printf ''{"locations":[{"file":"src/file.c","line":1,"column":6,"text":"void target(void) {}"}]}\n''',
    'else',
    '  printf ''{}\n''',
    'fi',
  ], executable)
  setfperm(executable, 'rwx------')

  var save_executable = g:readseek_executable
  var save_root_markers = g:readseek_root_markers
  g:readseek_executable = executable
  g:readseek_root_markers = ['.git']

  execute 'edit ' .. fnameescape(base .. '/project/src/file.c')
  cursor(1, 8)
  readseek#Definition()

  Check('definition start feedback', WaitForMessage('readseek.vim: finding definition...'))
  Check('definition completion feedback', WaitForMessage('readseek.vim: 1 definition found'))
  Check('definition jumped to result', expand('%:p') ==# base .. '/project/src/file.c' && line('.') == 1 && col('.') == 6)

  g:readseek_executable = save_executable
  g:readseek_root_markers = save_root_markers
  delete(base, 'rf')
enddef
def TestHoverLines()
  var lines = readseek#HoverLines({
    identifier: {text: 'target'},
    symbol: {kind: 'function', name: 'target'},
    file: 'src/file.c',
    line: 1,
    column: 6,
  })

  Check('hover identifier label', index(lines, 'identifier: target') >= 0)
  Check('hover symbol label', index(lines, 'symbol: target') >= 0)
  Check('hover kind label', index(lines, 'kind: function') >= 0)
  Check('hover location label', index(lines, 'location: src/file.c:1:6') >= 0)
enddef

def TestMap()
  Check('Map command exists', exists(':ReadSeekMap') == 2)
  Check('Map function exists', exists('*readseek#Map') == 1)
enddef

def TestInit()
  Check('Init command exists', exists(':ReadSeekInit') == 2)
  Check('Init function exists', exists('*readseek#Init') == 1)
  var init_plug = maparg('<Plug>(ReadSeekInit)', 'n', false, true)
  Check('init plug mapping', !empty(init_plug) && init_plug.rhs ==# '<ScriptCmd>ReadSeekInit<CR>')
enddef

def TestSearchLocations()
  var json = {
    results: [{
      file: 'src/main.rs',
      matches: [
        {
          start_line: 51,
          end_line: 64,
          hashlines: [{line: 51, hash: '56c', text: 'fn main() {'}],
        },
        {
          start_line: 70,
          end_line: 72,
          hashlines: [],
        },
      ],
    }],
  }
  var locations = readseek#SearchLocations(json, '/proj')
  Check('search location count', len(locations) == 2)
  Check('search uses start_line', locations[0].line == 51)
  Check('search text from first hashline', locations[0].text ==# 'fn main() {')
  Check('search resolves file', locations[0].file ==# '/proj/src/main.rs')
  Check('search empty hashlines text', locations[1].text ==# '')
  Check('search empty key yields nothing', empty(readseek#SearchLocations({}, '/proj')))
enddef

def TestDiagnosticsLocations()
  var locations = readseek#DiagnosticsLocations({
    diagnostics: [
      {kind: 'error', start_line: 4, end_line: 4},
      {kind: 'missing', start_line: 9, end_line: 11},
    ],
  }, '/project/file.rs')
  Check('diagnostic location count', len(locations) == 2)
  Check('diagnostic error location', locations[0].file ==# '/project/file.rs' && locations[0].line == 4)
  Check('diagnostic error text', locations[0].text ==# '[error] parser diagnostic')
  Check('diagnostic missing text', locations[1].text ==# '[missing] parser diagnostic')
enddef

def TestPluginConfiguration()
  Check('auto install defaults off', !g:readseek_auto_install)
  Check('auto open results defaults on', g:readseek_auto_open_results)
  Check('job timeout defaults positive', g:readseek_job_timeout_ms > 0)
  Check('check command exists', exists(':ReadSeekCheck') == 2)
  var check_plug = maparg('<Plug>(ReadSeekCheck)', 'n', false, true)
  Check('check plug mapping', !empty(check_plug) && check_plug.rhs ==# '<ScriptCmd>ReadSeekCheck<CR>')
enddef

def TestJobTimeout()
  var base = tempname()
  mkdir(base, 'p')
  var executable = base .. '/readseek-slow'
  writefile(['#!/bin/sh', 'sleep 1', 'printf ''{}\\n'''], executable)
  setfperm(executable, 'rwx------')

  var save_executable = g:readseek_executable
  var save_timeout = g:readseek_job_timeout_ms
  g:readseek_executable = executable
  g:readseek_job_timeout_ms = 25
  var done = false
  var callbacks = 0
  var timed_out = false
  readseek#job#RunRaw(['check', 'stdin:slow.rs'], '', (result: dict<any>) => {
    callbacks += 1
    timed_out = !result.ok && get(result, 'error', '') =~# 'timed out'
    done = true
  })

  Check('job timeout completes', WaitFor((): bool => done))
  sleep 100m
  Check('job timeout reports failure', timed_out)
  Check('job timeout invokes callback once', callbacks == 1)
  g:readseek_executable = save_executable
  g:readseek_job_timeout_ms = save_timeout
  delete(base, 'rf')
enddef

TestQuickfixItems()
TestResultLists()
TestMappings()
TestIdentifyArgs()
TestRootMarkers()
TestHealthCache()
TestReferenceFeedback()
TestRename()
TestRenameConflict()
TestRenameUnsupported()
TestRenameRequiresSavedBuffer()
TestDefinitionUsesIdentifier()
TestHoverLines()
TestMap()
TestInit()
TestSearchLocations()
TestDiagnosticsLocations()
TestPluginConfiguration()
TestJobTimeout()

if !empty(failures)
  writefile(failures, 'test-readseek-failures.log')
  cquit
endif

writefile(['ok'], 'test-readseek.log')
qa
