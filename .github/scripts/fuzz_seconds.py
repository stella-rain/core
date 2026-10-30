#!/usr/bin/env python3
"""The whole seconds of a fuzz-long `minutes` input (-max_total_time of libFuzzer).

GitHub hands a `type: number` input to the shell as text of any shape: a dispatch of 1 is `1.0`,
Candidate's 15 is `15`, a scheduled run falls back to `7`. Shell arithmetic takes only integers,
and libFuzzer reads -max_total_time=0 as no limit, so the conversion is here and refuses what it
cannot read.

  fuzz_seconds.py <minutes>    the seconds, rounded half up, on one line

A bad value prints nothing to stdout, `fuzz_seconds.py: <reason>` to stderr, and exits 1.
"""

import decimal
import re
import sys

NUMBER = re.compile(r"[0-9]+(\.[0-9]+)?")


def seconds(text):
    stripped = text.strip()
    if not NUMBER.fullmatch(stripped):
        raise ValueError(f"not a number: {text!r}")
    total = (decimal.Decimal(stripped) * 60).to_integral_value(rounding=decimal.ROUND_HALF_UP)
    if total < 1:
        raise ValueError(f"must be at least 1 second: {text!r}")
    return int(total)


def main(argv):
    if len(argv) != 2:
        print("usage: fuzz_seconds.py <minutes>", file=sys.stderr)
        return 1
    try:
        print(seconds(argv[1]))
    except ValueError as e:
        print(f"fuzz_seconds.py: {e}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
