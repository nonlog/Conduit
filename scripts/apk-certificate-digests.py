#!/usr/bin/env python3
"""Hash DER certificates from successful apksigner --print-certs-pem output.

The textual signer labels changed in Build Tools 37. PEM is independent of those
labels. Always run cryptographic `apksigner verify` successfully before this parser.
"""
import base64
import binascii
import hashlib
import re
import sys


def digests(report: str) -> list[str]:
    blocks = re.findall(r"-----BEGIN CERTIFICATE-----\s*(.*?)\s*-----END CERTIFICATE-----", report, re.S)
    if not blocks:
        raise ValueError("apksigner did not emit any signing certificate PEM")
    result = set()
    for block in blocks:
        der = base64.b64decode(re.sub(r"\s+", "", block), validate=True)
        if not der:
            raise ValueError("empty signing certificate")
        result.add(hashlib.sha256(der).hexdigest())
    return sorted(result)


def self_test() -> None:
    def pem(value: bytes) -> str:
        return "-----BEGIN CERTIFICATE-----\n" + base64.b64encode(value).decode() + "\n-----END CERTIFICATE-----\n"
    a, b = pem(b"synthetic-certificate-a"), pem(b"synthetic-certificate-b")
    assert digests("Signer #1\n" + a) == digests("V3.0 Signer:\n" + a + "V2 Signer:\n" + a)
    assert len(digests(a + b)) == 2
    try:
        digests("Verifies\n")
    except ValueError:
        pass
    else:
        raise AssertionError("Missing cert must fail closed")
    print("SIGNER_PEM_PARSER_TESTS_PASSED")


if __name__ == "__main__":
    if sys.argv[1:] == ["--self-test"]:
        self_test()
    elif sys.argv[1:]:
        raise SystemExit("Usage: apk-certificate-digests.py [--self-test]")
    else:
        try:
            print("\n".join(digests(sys.stdin.read())))
        except (ValueError, binascii.Error) as error:
            raise SystemExit(str(error)) from error
