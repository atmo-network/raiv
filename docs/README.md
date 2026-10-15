# Documentation index

Living index of project design documentation. The [backlog](../BACKLOG.md) owns remaining work; the [changelog](../CHANGELOG.md) owns completed outcomes. The [project entrypoint](../README.md) describes current APIs/support boundaries, not implementation history.

## Documents

- [Raiv 0.0.2 specification](raiv.spec.en.md): Normative portable-library, lifecycle, isolated WASM guest/native-host ABI, minimal terminal frontend, inference and availability requirements. This is a design contract, not a report of implemented behavior.
- [DASL Content IDs](cid.spec.en.md): Restricted CID profile, parsing and IPFS relationship; upstream snapshot dated 2026-10-01.
- [DRISL](drisl.spec.en.md): Deterministic CBOR representation and tag-42 links; upstream snapshot dated 2026-10-01.
- [CAR](car.spec.en.md): Content-addressable archive framing, headers and conformance; upstream snapshot dated 2026-10-01.

## Snapshot provenance and integrity

The three external specifications were converted locally from the previously pinned HTML responses at https://dasl.ing/cid.html, https://dasl.ing/drisl.html and https://dasl.ing/car.html, without downloading newer revisions. Metadata, normative text, ordered/nested procedures, examples and reference links are retained. Conversion changes representation, not the Raiv wire profile. Original source-response SHA-256 values are preserved here for provenance; the HTML files are no longer the stored representation:

```text
ef18188440328d83f15835fc780dde5288b80dc6f3a9c458c18319b5202782ef  cid.html
dd7ce780187e17879da82d9363841fb513a65727b615a0cdf09af7796468b818  drisl.html
7e77f96927a7a5ee352542d9a3ba3d762298b450b9c7c0d3a363026661837ee4  car.html
```

By owner decision, documentation snapshots have no separate checksum manifest or checksum validation gate. The source-response hashes above and pinned fixture provenance remain attribution/integrity evidence, not a new representation checksum policy, canonical agent root or state-format vector.

Upstream CBOR fixtures and their MIT license remain at `fixtures/dasl-testing/`, pinned to [commit b6ea320b07b385c417aaae265425ec334a3fd70c](https://github.com/hyphacoop/dasl-testing/tree/b6ea320b07b385c417aaae265425ec334a3fd70c). Preserve source attribution and license; project licensing does not replace upstream notices. Integrity checks alone are not decoder conformance tests.

## Related

- [Conformance fixture profile](../fixtures/README.md)
- [Library package entrypoint](../lib/README.md)
- [Project entrypoint](../README.md)
