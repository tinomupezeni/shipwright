use shipwright_common::version;

pub fn run() {
    println!("{}", version::get_detailed_version());
    println!("\nFor more information, visit: https://github.com/tinomupezeni/shipwright");
}
