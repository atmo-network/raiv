use super::*;
use crate::tests::activation::{admission, output};
use ciborium::value::Value;

fn limits() -> ManifestLimits {
    ManifestLimits {
        wire: DecodeLimits::default(),
        max_blocks: 100,
        max_admissions: 10,
    }
}

fn captured() -> CheckpointManifest {
    let module = output::invalid_output_module();
    let registry = admission::registry(&module);
    let registered = registry.agent([0xb0; 32]).unwrap();
    let mut book = AdmissionBook::new(3).unwrap();
    let input = admission::input(0x60, 0x70);
    let grant = input.budget_admission();
    book.admit_actor_wake(registered, input, grant).unwrap();
    CheckpointManifest::capture(registered, &book, &limits()).unwrap()
}

fn bytes(value: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    ciborium::ser::into_writer(value, &mut out).unwrap();
    out
}

fn array(value: &mut Value) -> &mut Vec<Value> {
    let Value::Array(a) = value else { panic!() };
    a
}

#[test]
fn exact_closed_roundtrip_independent_oracle_full_u64_and_explicit_caps() {
    let mut data = captured();
    data.resources.fuel_per_step = u64::MAX;
    data.admissions[0].grant.activation_units = u64::MAX;
    let encoded = data.encode(&limits()).unwrap();
    let decoded = CheckpointManifest::decode(&encoded, &limits()).unwrap();
    assert_eq!(decoded.encode(&limits()).unwrap(), encoded);
    assert_eq!(decoded.resources().fuel_per_step, u64::MAX);
    assert_eq!(decoded.admissions()[0].grant.activation_units, u64::MAX);
    let value: Value = ciborium::de::from_reader(encoded.as_slice()).unwrap();
    assert_eq!(bytes(&value), encoded);
    for caps in [
        ManifestLimits {
            max_blocks: 0,
            ..limits()
        },
        ManifestLimits {
            max_admissions: 0,
            ..limits()
        },
        ManifestLimits {
            wire: DecodeLimits {
                max_block_bytes: 1,
                ..DecodeLimits::default()
            },
            ..limits()
        },
    ] {
        assert!(data.encode(&caps).is_err());
        assert!(CheckpointManifest::decode(&encoded, &caps).is_err());
    }
}

#[test]
fn version_shape_cid_order_status_null_counters_and_binding_mutations_are_not_accepted() {
    let encoded = captured().encode(&limits()).unwrap();
    let original: Value = ciborium::de::from_reader(encoded.as_slice()).unwrap();
    for mutation in 0..15 {
        let mut v = original.clone();
        let a = array(&mut v);
        match mutation {
            0 => a[1] = Value::Integer(2.into()),
            1 => a.push(Value::Null),
            2 => a[2] = Value::Bytes(vec![0; 31]),
            3 => a[7] = Value::Text("replicated".into()),
            4 => array(&mut a[6])[0] = Value::Integer(0.into()),
            5 => array(&mut a[6])[1] = Value::Integer(0.into()),
            6 => {
                let c = array(&mut a[8]);
                c.reverse();
            }
            7 => {
                let c = array(&mut a[8]);
                c.push(c[0].clone());
            }
            8 => array(&mut array(&mut a[9])[0])[2] = Value::Integer(4.into()),
            9 => array(&mut array(&mut a[9])[0])[3] = Value::Integer(1.into()),
            10 => array(&mut array(&mut a[9])[0])[4] = a[3].clone(),
            11 => {
                let records = array(&mut a[9]);
                records.push(records[0].clone());
            }
            12 => array(&mut array(&mut array(&mut a[9])[0])[0])[1] = Value::Bytes(vec![0; 32]),
            13 => array(&mut array(&mut array(&mut a[9])[0])[0])[3] = Value::Integer(1.into()),
            14 => {
                a[3] = Value::Bytes(
                    raiv::format::DaslCid::for_block(raiv::format::DaslCodec::Raw, &[0xa0])
                        .as_bytes()
                        .to_vec(),
                )
            }
            _ => unreachable!(),
        }
        assert!(
            CheckpointManifest::decode(&bytes(&v), &limits()).is_err(),
            "mutation {mutation}"
        );
    }
    let mut trailing = encoded.clone();
    trailing.push(0);
    assert!(CheckpointManifest::decode(&trailing, &limits()).is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn indexed_genesis_restoration_missing_extras_and_foreign_registry_metadata_never_approve_or_install(
) {
    use crate::disk::{tests::Directory, DiskBlockStore};
    let module = output::invalid_output_module();
    let registry = admission::registry(&module);
    let registered = registry.agent([0xb0; 32]).unwrap();
    let mut book = AdmissionBook::new(2).unwrap();
    let input = admission::input(0x60, 0x70);
    let grant = input.budget_admission();
    book.admit_actor_wake(registered, input, grant).unwrap();
    let data = CheckpointManifest::capture(registered, &book, &limits()).unwrap();
    let directory = Directory::new();
    let mut disk = DiskBlockStore::open_existing(&directory.0, DecodeLimits::default()).unwrap();
    disk.place(registered.snapshot()).unwrap();
    let encoded = data.encode(&limits()).unwrap();
    disk.manifest(limits().wire)
        .replace(None, &encoded)
        .unwrap();
    drop(disk);
    let mut disk = DiskBlockStore::open_existing(&directory.0, DecodeLimits::default()).unwrap();
    let encoded = disk.manifest(limits().wire).read().unwrap().unwrap();
    let mut decoded = CheckpointManifest::decode(&encoded, &limits()).unwrap();
    assert_eq!(
        &decoded
            .restore(&disk, &raiv::limits::Limits::default())
            .unwrap(),
        registered.snapshot()
    );
    let original = decoded.required.clone();
    let missing = original[0];
    std::fs::remove_file(directory.0.join(missing.to_string())).unwrap();
    assert!(decoded
        .restore(&disk, &raiv::limits::Limits::default())
        .is_err());
    disk.put(
        missing,
        registered
            .snapshot()
            .blocks()
            .find(|(cid, _)| *cid == missing)
            .unwrap()
            .1,
    )
    .unwrap();
    let extra = DaslCid::for_block(raiv::format::DaslCodec::Drisl, &[0xa0]);
    disk.put(extra, &[0xa0]).unwrap();
    decoded.required.push(extra);
    decoded
        .required
        .sort_unstable_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    assert!(matches!(
        decoded.restore(&disk, &raiv::limits::Limits::default()),
        Err(CheckpointError::Binding)
    ));
    decoded.required = original;
    let foreign = admission::registry(&module);
    assert!(
        CheckpointManifest::capture(foreign.agent([0xb0; 32]).unwrap(), &book, &limits()).is_err()
    );
    assert_eq!(registry.agent([0xb0; 32]).unwrap().anchor(), data.anchor());
}
