# Security policy

## Scope

In scope: the code in this repository -- `indexer`, `scorer`, `bot`, `crawler`, the shared
libraries, migrations, and CI/deployment config. Anything that could lead to a wrong number
being shown, a read-only guarantee being broken, a secret being logged or leaked, or -- once
the wallet/custody work lands -- an unsigned transaction being built incorrectly or a key
being handled where the design says it never should be.

Out of scope: Telegram's own platform and client apps, third-party RPC/Geyser providers,
vulnerabilities in a dependency that are already tracked upstream (file those with the
dependency; `deny.toml` is how this repo tracks which ones affect it and why), and anything
requiring physical access to a deployed host.

As of this writing the running system holds no private keys and moves no funds -- see the
README. A report against a build that adds device-held wallet custody and real transaction
signing is very much in scope, and the most valuable kind: the trust boundary between what
the backend builds and what the device signs is the part of this project most worth breaking.

## Reporting

Do not open a public issue for a suspected vulnerability. Report it privately:

- GitHub: use "Report a vulnerability" under this repository's Security tab (private
  Security Advisory), or
- Email: cristiando0902@gmail.com, subject line starting `SECURITY:`.

Include what you found, the steps to reproduce it, and what you think the impact is. A
working proof of concept is useful; a full writeup is not required to start a conversation.

## What to expect

This is a small, single-operator project, not a company with a security team -- a report gets
a real reply from the person who reads it, not an automated ticket. Expect an acknowledgment
within a few days. There is no bug bounty; credit in the eventual fix's changelog, on request,
is what's on offer. A confirmed vulnerability gets fixed and disclosed once a fix is out, with
the reporter kept in the loop throughout rather than told to wait silently.
