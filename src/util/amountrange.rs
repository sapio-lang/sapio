use bitcoin::util::amount::Amount;

#[derive(Clone, Copy)]
pub struct AmountRange {
    min: Amount,
    max: Amount,
}
impl AmountRange {
    pub fn new() -> AmountRange {
        AmountRange {
            min: Amount::max_value(),
            max: Amount::min_value(),
        }
    }
    pub fn update_range(&mut self, amount: Amount) {
        self.min = std::cmp::min(self.min, amount);
        self.max = std::cmp::max(self.max, amount);
    }
}
