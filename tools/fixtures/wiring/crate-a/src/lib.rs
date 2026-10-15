pub fn used_elsewhere() {}

pub fn used_only_in_test() {}

pub fn never_used() {}

#[cfg(test)]
mod tests {
    fn t() {
        super::used_only_in_test();
    }
}
