# SPDX-License-Identifier: LGPL-2.1-or-later
# Copyright (c) 2026 Jarkko Sakkinen

PREFIX ?= /usr/local
BINDIR ?= $(PREFIX)/bin
MANDIR ?= $(PREFIX)/share/man
CARGO ?= cargo
INSTALL ?= install
GH ?= gh
NODE ?= node
NPM ?= npm

.PHONY: all check clippy publish install uninstall clean

all:
	$(CARGO) build

check:
	$(CARGO) build
	$(CARGO) clippy --all-targets

clippy:
	$(CARGO) clippy --all-targets

publish:
	CARGO="$(CARGO)" GH="$(GH)" NODE="$(NODE)" NPM="$(NPM)" \
		./scripts/publish.sh "$(VERSION)"

install:
	$(CARGO) build --release
	$(INSTALL) -d "$(DESTDIR)$(BINDIR)" "$(DESTDIR)$(MANDIR)/man1"
	$(INSTALL) -m 755 target/release/readseek "$(DESTDIR)$(BINDIR)/readseek"
	$(INSTALL) -m 644 man/man1/readseek.1 "$(DESTDIR)$(MANDIR)/man1/readseek.1"

uninstall:
	$(RM) "$(DESTDIR)$(BINDIR)/readseek" "$(DESTDIR)$(MANDIR)/man1/readseek.1"

clean:
	$(CARGO) clean
