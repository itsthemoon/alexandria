use std::io::Read;
fn main() {
    let mut s = String::new();
    std::io::stdin().read_to_string(&mut s).unwrap();
    println!("{}", alexandria_zim::html_to_text(&s));
}
