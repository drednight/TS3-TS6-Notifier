fn main() {
    println!("tsn-native bootstrap ok");
}

#[cfg(test)]
mod tests {
    #[test]
    fn boot() {
        assert_eq!(1 + 1, 2);
    }
}
