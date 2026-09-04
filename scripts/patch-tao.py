# -*- coding: utf-8 -*-
"""Patch vendor/tao (line-based, CRLF-safe): transparent windows use
WS_EX_NOREDIRECTIONBITMAP instead of legacy DWM blur-behind glass."""
import io

P = r"D:\My things\Learn\高二\VCC\vendor\tao-0.35.3\src\platform_impl\windows\window.rs"
raw = io.open(P, "rb").read()
crlf = b"\r\n" in raw
nl = "\r\n" if crlf else "\n"
lines = raw.decode("utf-8").split(nl)

out = []
hit_flags = 0
hit_blur = 0
for line in lines:
    # 1) NO_BACK_BUFFER flag: enable for all transparent windows
    if line.strip() == "pl_attribs.no_redirection_bitmap,":
        indent = line[: len(line) - len(line.lstrip())]
        line = indent + "pl_attribs.no_redirection_bitmap || attributes.transparent,"
        hit_flags += 1
    # 2) skip the legacy DwmEnableBlurBehindWindow call for transparent windows
    stripped = line.strip()
    if stripped == "if attributes.transparent && !pl_attribs.no_redirection_bitmap {":
        line = line.replace(
            "!pl_attribs.no_redirection_bitmap",
            "!(pl_attribs.no_redirection_bitmap || attributes.transparent)",
        )
        hit_blur += 1
    out.append(line)

assert hit_flags == 1, "flags line not found: %d" % hit_flags
assert hit_blur == 1, "blur line not found: %d" % hit_blur
io.open(P, "wb").write(nl.join(out).encode("utf-8"))
print("patched OK (flags=%d blur=%d crlf=%s)" % (hit_flags, hit_blur, crlf))
