pub mod modules;

fn main() {
    let _app = modules::router();

    let name = String::from("Andrei");
    needs_static(name);
}

fn needs_static<T: 'static>(value: T) {}
