const CHECKPOINT_PERIOD: usize = 8;

use std::fmt;
mod block;
mod skip_index;
mod skip_index_builder;

use crate::DocId;

pub use self::skip_index::SkipIndex;
pub use self::skip_index_builder::SkipIndexBuilder;

/// A checkpoint contains meta-information about
/// a block. Either a block of documents, or another block
/// of checkpoints.
///
/// All of the intervals here defined are semi-open.
/// The checkpoint describes that the block within the bytes
/// `[start_offset..end_offset)` spans over the docs
/// `[start_doc..end_doc)`.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct Checkpoint {
    pub start_doc: DocId,
    pub end_doc: DocId,
    pub start_offset: u64,
    pub end_offset: u64,
}

impl fmt::Debug for Checkpoint {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "(doc=[{}..{}), bytes=[{}..{}))",
            self.start_doc, self.end_doc, self.start_offset, self.end_offset
        )
    }
}

#[cfg(test)]
mod tests {

    use std::io;

    use proptest::strategy::{BoxedStrategy, Strategy};

    use crate::directory::OwnedBytes;
    use crate::store::index::Checkpoint;
    use crate::DocId;

    use super::{SkipIndex, SkipIndexBuilder};

    #[test]
    fn test_skip_index_empty() -> io::Result<()> {
        let mut output: Vec<u8> = Vec::new();
        let skip_index_builder: SkipIndexBuilder = SkipIndexBuilder::new();
        skip_index_builder.write(&mut output)?;
        let skip_index: SkipIndex = SkipIndex::from(OwnedBytes::new(output));
        let mut skip_cursor = skip_index.checkpoints();
        assert!(skip_cursor.next().is_none());
        Ok(())
    }

    #[test]
    fn test_skip_index_single_el() -> io::Result<()> {
        let mut output: Vec<u8> = Vec::new();
        let mut skip_index_builder: SkipIndexBuilder = SkipIndexBuilder::new();
        let checkpoint = Checkpoint {
            start_doc: 0,
            end_doc: 2,
            start_offset: 0,
            end_offset: 3,
        };
        skip_index_builder.insert(checkpoint);
        skip_index_builder.write(&mut output)?;
        let skip_index: SkipIndex = SkipIndex::from(OwnedBytes::new(output));
        let mut skip_cursor = skip_index.checkpoints();
        assert_eq!(skip_cursor.next(), Some(checkpoint));
        assert_eq!(skip_cursor.next(), None);
        Ok(())
    }

    #[test]
    fn test_skip_index() -> io::Result<()> {
        let mut output: Vec<u8> = Vec::new();
        let checkpoints = vec![
            Checkpoint {
                start_doc: 0,
                end_doc: 3,
                start_offset: 4,
                end_offset: 9,
            },
            Checkpoint {
                start_doc: 3,
                end_doc: 4,
                start_offset: 9,
                end_offset: 25,
            },
            Checkpoint {
                start_doc: 4,
                end_doc: 6,
                start_offset: 25,
                end_offset: 49,
            },
            Checkpoint {
                start_doc: 6,
                end_doc: 8,
                start_offset: 49,
                end_offset: 81,
            },
            Checkpoint {
                start_doc: 8,
                end_doc: 10,
                start_offset: 81,
                end_offset: 100,
            },
        ];

        let mut skip_index_builder: SkipIndexBuilder = SkipIndexBuilder::new();
        for &checkpoint in &checkpoints {
            skip_index_builder.insert(checkpoint);
        }
        skip_index_builder.write(&mut output)?;

        let skip_index: SkipIndex = SkipIndex::from(OwnedBytes::new(output));
        assert_eq!(
            &skip_index.checkpoints().collect::<Vec<_>>()[..],
            &checkpoints[..]
        );
        Ok(())
    }

    fn offset_test(doc: DocId) -> u64 {
        (doc as u64) * (doc as u64)
    }

    #[test]
    fn test_skip_index_long() -> io::Result<()> {
        let mut output: Vec<u8> = Vec::new();
        let checkpoints: Vec<Checkpoint> = (0..1000)
            .map(|i| Checkpoint {
                start_doc: i,
                end_doc: i + 1,
                start_offset: offset_test(i),
                end_offset: offset_test(i + 1),
            })
            .collect();
        let mut skip_index_builder = SkipIndexBuilder::new();
        for checkpoint in &checkpoints {
            skip_index_builder.insert(*checkpoint);
        }
        skip_index_builder.write(&mut output)?;
        assert_eq!(output.len(), 4035);
        let resulting_checkpoints: Vec<Checkpoint> = SkipIndex::from(OwnedBytes::new(output))
            .checkpoints()
            .collect();
        assert_eq!(&resulting_checkpoints, &checkpoints);
        Ok(())
    }

    fn integrate_delta(mut vals: Vec<u64>) -> Vec<u64> {
        let mut prev = 0u64;
        for val in vals.iter_mut() {
            let new_val = *val + prev;
            prev = new_val;
            *val = new_val;
        }
        vals
    }

    // Generates a sequence of n valid checkpoints, with n < max_len.
    fn monotonic_checkpoints(max_len: usize) -> BoxedStrategy<Vec<Checkpoint>> {
        (1..max_len)
            .prop_flat_map(move |len: usize| {
                (
                    proptest::collection::vec(1u64..20u64, len as usize).prop_map(integrate_delta),
                    proptest::collection::vec(1u64..26u64, len as usize).prop_map(integrate_delta),
                )
                    .prop_map(|(docs, offsets)| {
                        (0..docs.len() - 1)
                            .map(move |i| Checkpoint {
                                start_doc: docs[i] as DocId,
                                end_doc: docs[i + 1] as DocId,
                                start_offset: offsets[i],
                                end_offset: offsets[i + 1],
                            })
                            .collect::<Vec<Checkpoint>>()
                    })
            })
            .boxed()
    }

    fn seek_manual<I: Iterator<Item = Checkpoint>>(
        checkpoints: I,
        target: DocId,
    ) -> Option<Checkpoint> {
        checkpoints
            .into_iter()
            .filter(|checkpoint| checkpoint.end_doc > target)
            .next()
    }

    fn test_skip_index_aux(skip_index: SkipIndex, checkpoints: &[Checkpoint]) {
        if let Some(last_checkpoint) = checkpoints.last() {
            for doc in 0u32..last_checkpoint.end_doc {
                let expected = seek_manual(skip_index.checkpoints(), doc);
                assert_eq!(expected, skip_index.seek(doc), "Doc {}", doc);
            }
            assert!(skip_index.seek(last_checkpoint.end_doc).is_none());
        }
    }

    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(20))]
        #[test]
        fn test_proptest_skip(checkpoints in monotonic_checkpoints(100)) {
             let mut skip_index_builder = SkipIndexBuilder::new();
             for checkpoint in checkpoints.iter().cloned() {
                 skip_index_builder.insert(checkpoint);
             }
             let mut buffer = Vec::new();
             skip_index_builder.write(&mut buffer).unwrap();
             let skip_index = SkipIndex::from(OwnedBytes::new(buffer));
             let iter_checkpoints: Vec<Checkpoint> = skip_index.checkpoints().collect();
             assert_eq!(&checkpoints[..], &iter_checkpoints[..]);
             test_skip_index_aux(skip_index, &checkpoints[..]);
         }
    }
}
