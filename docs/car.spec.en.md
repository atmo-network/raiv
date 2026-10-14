# Content-Addressable aRchives (CAR)

- **Date:** 2026-10-01
- **Status:** stable
- **Editors:** [Robin Berjon](https://berjon.com/) <[robin@berjon.com](mailto:robin@berjon.com)>; [Juan Caballero](https://bumblefudge.com/) <[bumblefudge@learningproof.xyz](mailto:bumblefudge@learningproof.xyz)>
- **Issues:** [list](https://tangled.org/dasl.ing/dasl.ing/issues), [new](https://tangled.org/dasl.ing/dasl.ing/issues/new)

## Abstract

The CAR format offers a serialized representation of set of content-addressed resources in one single concatenated stream, alongside a header that describes that content.

## Introduction

The CAR format (Content Addressable aRchives) is used to store series of content-addressable objects as a sequence of bytes. It packages that stream of objects with a header.

Much of the content of this specification was initially developed as part of the [IPLD](https://github.com/ipld/ipld) project. This specification was developed based on demand from the community to have just the one simplified document. Note that a CARv2 specification was developed at some point to add support for an index trailer, but it met with limited adoption and so was not considered when bringing CAR into DASL.

## Parsing CAR

The CAR format is made of a Header followed by a Body. The Header is a length-prefixed chunk of DRISL ([[drisl](#drisl)]) and the Body is a sequence of zero or more length-prefixed blocks that contain a tuple of a DASL CID ([[cid](#cid)]) which is always 36 bytes long and the data addressed by that CID. The length prefix in a CAR is encoded as an unsigned variable-length integer ([[varint](#varint)], a variant of [LEB128](https://en.wikipedia.org/wiki/LEB128)). This integer specifies the number of remaining bytes, excluding the bytes used to encode the integer.

```text
|--- Header --| |------------------- Body -------------------|
[ int | DRISL ] [ int | CID | data ] [ int | CID | data ] …
```

### Parse a CAR

The steps to **parse a CAR** are:

1. Accept a byte stream _bytes_ that is consumed with every step that reads from it.
2. Run the steps to [parse a CAR header](#parse-a-car-header) with _bytes_ to obtain _metadata_.
3. Set up array _blocks_ and run these substeps:
   1. If _bytes_ is empty, terminate these substeps.
   2. Run the steps to [parse a CAR block](#parse-a-car-block) with _bytes_ to obtain _cid_, _data size_, and _data_.
   3. Push an entry onto _blocks_ containing _cid_, _data size_, and _data_.
   4. Return to the beginning of these substeps.
4. Return _metadata_ and _blocks_.

Note that the CAR header contains a near-arbitrary DRISL object that is to be treated as metadata ([[drisl](#drisl)]). For historical reasons, there are two constraints on the header:

- The object must contain a `version` map entry, the value of which is always integer-type `1`. Version numbers in data formats are an anti-pattern, and as a result this number is guaranteed never to change.
- The object must contain a `roots` entry, which must be of type array. It may be empty, but if it isn't then it must be an array of CIDs encoded using tag 42 ([[cid](#cid)]). A CAR can be used to contain one or more DAGs of [[drisl](#drisl)] content and the purpose of the `roots` is to list one or more roots for those DAGs. The array may be empty if you do not care about encoding DAGs.

Some historical implementations will only return _version_ and _roots_, but it is recommended that they make the entire _metadata_ object available. A best practice for authors is to use the _metadata_ to capture MASL content, which is able to provide metadata and a pathing mapping for the entire content of the CAR stream if needed ([[masl](#masl)]).

### Parse a CAR header

The steps to **parse a CAR header** are:

1. Accept a byte stream _bytes_.
2. Read an unsigned varint _length_ from _bytes_ ([[varint](#varint)]).
3. If _length_ is 0, throw an error.
4. Read _length_ bytes from _bytes_ and decode them as DRISL ([[drisl](#drisl)]) into _metadata_. If _metadata_ is not a map, throw an error.
5. If _metadata_ does not have a `version` key entry with integer value `1`, throw an error. Otherwise, store `version` in _version_.
6. If _metadata_ does not have a `roots` key entry that is an array, or if that array contains anything other than DASL CIDs, throw an error. Otherwise, store `roots` in _roots_.
7. Return _metadata_. (For implementations that only report `version` and `roots`, return those.)

After its header, CAR contains a series of blocks each of which is length-prefixed.

### Parse a CAR block

The steps to **parse a CAR block** are:

1. Accept a byte stream _bytes_.
2. Read an unsigned varint _length_ from _bytes_ ([[varint](#varint)]).
3. If _length_ is less than 36, throw an error.
4. Read a CID ([[cid](#cid)]) from _bytes_ and store it in _cid_. Note: the length of the CID is always 36 bytes.
5. Set _data size_ to _length_ minus 36 (the CID length).
6. Read _data size_ bytes from _bytes_ and store the result in _data_.
7. Return _cid_, _data size_, and _data_.

## Additional Considerations

### Conformance

A CAR stream must only feature DASL CIDs.

A CAR stream must have CIDs that match the data body that follows them. A CAR implementation should verify that CIDs match block body data, though it may delegate verification to other components. (Keep in mind that not verifying at all negates the value of content addressing.)

A CAR stream's stated roots must match CIDs contained in the Body. However, implementations frequently operate in a streaming fashion such that they have no way of knowing whether a CAR stream conforms to this requirement before having processed the entire stream. Checking correctness with respect to this requirement may therefore be more readily performed via a warning (at end of processing) or a dedicated validator.

### Determinism

Deterministic CAR creation is not covered by this specification. However, deterministic generation of a CAR from a given graph is possible and is relied upon by certain uses of the format, most notably, [Filecoin](https://filecoin-project.github.io/specs). dCAR may be the topic of a future specification.

Care regarding the ordering of the `roots` array in the Header and avoidance of duplicate blocks may also be required for strict determinism.

### Security & Verifiability

The `roots` specified by the Header of a CAR is expected to appear somewhere in its Body section, however there is no requirement that the roots define entire DAGs, nor that all blocks in a CAR must be part of DAGs described by the root CIDs in the Header. Therefore, the `roots` must not be used alone to determine or differentiate the contents of a CAR.

The CAR format contains no internal means, beyond the blocks and their CIDs, to verify or differentiate contents. Where such a requirement exists, this must be performed externally, such as creating a digest of the entire CAR (and refer to it using a CID).

## Appendix: Media Type

The media type for CAR is [`application/vnd.ipld.car`](https://www.iana.org/assignments/media-types/application/vnd.ipld.car).

The conventional file extension for CAR is `.car`.

## References

### [cid]

Robin Berjon & Juan Caballero. [Content IDs (CIDs)](https://dasl.ing/cid.html). 2026-10-01. URL: <https://dasl.ing/cid.html>

### [drisl]

Robin Berjon & Juan Caballero. [DRISL — Deterministic Representation for Interoperable Structures & Links](https://dasl.ing/drisl.html). 2026-10-01. URL: <https://dasl.ing/drisl.html>

### [masl]

Robin Berjon & Juan Caballero. [MASL — Metadata for Arbitrary Structures & Links](https://dasl.ing/masl.html). 2026-10-01. URL: <https://dasl.ing/masl.html>

### [varint]

[unsigned varint](https://github.com/multiformats/unsigned-varint). URL: <https://github.com/multiformats/unsigned-varint>
