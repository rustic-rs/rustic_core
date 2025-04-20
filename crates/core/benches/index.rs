//! Index benchmarks

use criterion::{Criterion, criterion_group, criterion_main};
use rand::{SeedableRng, rngs::StdRng};
use rustic_core::{
    Id,
    index::{
        ReadIndex,
        sorted::{IndexCollector, IndexType},
    },
    repofile::{BlobType, IndexBlob, IndexPack},
};

/// Benchmark index access
fn bench_index(c: &mut Criterion) {
    let mut rng = StdRng::seed_from_u64(5);

    let packs: Vec<_> = (0..100)
        .map(|_| IndexPack {
            id: Id::random_from_rng(&mut rng).into(),
            blobs: (0..10_000)
                .map(|_| IndexBlob {
                    id: Id::random_from_rng(&mut rng).into(),
                    tpe: BlobType::Data,
                    offset: 0,
                    length: 0,
                    uncompressed_length: None,
                })
                .collect(),
            ..Default::default()
        })
        .collect();

    let find_id = Id::random_from_rng(&mut rng).into();

    let mut collector = IndexCollector::new(IndexType::DataIds);
    collector.extend(packs);
    let index = collector.into_index();
    let _ = c.bench_function("test", |b| {
        b.iter(|| {
            _ = index.has(BlobType::Data, &find_id);
        });
    });
}

criterion_group!(benches, bench_index);
criterion_main!(benches);
