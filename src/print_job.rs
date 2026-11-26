use crate::core::MAX_BATCH;
use std::ops::Range;

#[derive(Clone, Copy, PartialEq)]
pub enum PrintScope {
    Current(usize),
    Batch,
    Range,
}

pub struct PrintJob {
    pub records: Range<usize>,
    pub copies: usize,
}

impl PrintJob {
    pub fn new(scope: PrintScope, records: usize, copies: usize) -> Option<Self> {
        match scope {
            PrintScope::Range => Some(Self {
                records: 0..1,
                copies: 1,
            }),
            PrintScope::Current(index) if index < records => Some(Self {
                records: index..index + 1,
                copies: 1,
            }),
            PrintScope::Batch if (1..=MAX_BATCH).contains(&records.checked_mul(copies)?) => {
                Some(Self {
                    records: 0..records,
                    copies,
                })
            }
            _ => None,
        }
    }

    pub fn len(&self) -> usize {
        self.records.len() * self.copies
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proof_selects_one_current_record_even_with_invalid_copy_count() {
        let proof = PrintJob::new(PrintScope::Current(2), 4, 0).unwrap();
        assert_eq!(proof.records, 2..3);
        assert_eq!(proof.len(), 1);
        assert!(PrintJob::new(PrintScope::Current(4), 4, 1).is_none());
    }

    #[test]
    fn batch_limits_apply_to_physical_labels() {
        assert_eq!(
            PrintJob::new(PrintScope::Batch, 5, 100).unwrap().len(),
            MAX_BATCH
        );
        for (records, copies) in [(0, 1), (1, 0), (5, 101), (2, usize::MAX)] {
            assert!(PrintJob::new(PrintScope::Batch, records, copies).is_none());
        }
    }
}
