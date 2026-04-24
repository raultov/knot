//! Sample Rust file for testing knot's Rust language support.
//! This file contains comprehensive examples of Rust constructs.

/// Represents a simple counter with an optional label.
pub struct Counter {
    /// Current count value
    count: u32,
    /// Optional label for display
    label: Option<String>,
}

/// A basic trait for incrementable entities.
trait Incrementable {
    /// Increment the value by 1
    fn increment(&mut self);

    /// Reset to initial state
    fn reset(&mut self);
}

/// Implementation of Incrementable for Counter
impl Incrementable for Counter {
    fn increment(&mut self) {
        self.count += 1;
    }

    fn reset(&mut self) {
        self.count = 0;
    }
}

/// Inherent implementation block for Counter
impl Counter {
    /// Create a new Counter with given initial value
    pub fn new(initial: u32) -> Self {
        Counter {
            count: initial,
            label: None,
        }
    }

    /// Create a Counter with a label
    pub fn with_label(initial: u32, label: &str) -> Self {
        Counter {
            count: initial,
            label: Some(label.to_string()),
        }
    }

    /// Get the current count value
    pub fn get_count(&self) -> u32 {
        self.count
    }

    /// Add a value to the counter
    pub fn add(&mut self, value: u32) {
        self.count += value;
    }
}

/// Represents a color enum with RGB values.
enum Color {
    Red,
    Green,
    Blue,
    Custom(u8, u8, u8),
}

/// A union type for demonstration (rarely used in practice).
union MaybeFloat {
    as_float: f64,
    as_bytes: [u8; 8],
}

/// Type alias for a callback function
type Callback = fn(u32) -> u32;

/// Constant definition
const MAX_SIZE: usize = 1024;

/// Static mutable counter (use with caution)
static mut COUNTER: u32 = 0;

/// Generic function that adds two values of any type that implements Add.
fn add<T>(a: T, b: T) -> T
where
    T: std::ops::Add<Output = T>,
{
    a + b
}

/// Function with lifetime parameters
fn longest<'a>(s1: &'a str, s2: &'a str) -> &'a str {
    if s1.len() > s2.len() {
        s1
    } else {
        s2
    }
}

/// Generic struct with trait bounds
struct Container<T: Clone> {
    value: T,
}

impl<T: Clone> Container<T> {
    fn get(&self) -> T {
        self.value.clone()
    }
}

/// Macro for creating a vector with initial values
macro_rules! init_vec {
    ($($val:expr),*) => {
        vec![$($val),*]
    };
}

/// Another macro with different pattern
macro_rules! count_items {
    ($($item:expr),*) => {
        vec![$(($item, 1)),*].len()
    };
}

/// Inner module demonstrating module hierarchy
mod inner {
    //! Inner module documentation

    /// Public function in inner module
    pub fn inner_function() -> String {
        "Hello from inner".to_string()
    }
}

/// Re-export macro from inner module
pub use inner::inner_function;

/// Documentation comment on a function
///
/// # Arguments
/// * `value` - The input value to process
///
/// # Returns
/// The processed value after transformation
fn process_value(value: u32) -> u32 {
    // Call the println macro
    println!("Processing: {}", value);

    // Use init_vec macro
    let numbers = init_vec![1, 2, 3, 4, 5];

    // Access static mut (unsafe but for demonstration)
    unsafe {
        COUNTER += 1;
    }

    // Return processed value
    value * 2
}

/// Async function example
async fn fetch_data() -> Result<String, &'static str> {
    Ok("data".to_string())
}

/// Trait with generic associated type
trait Repository {
    type Item;
    fn get(&self, id: u32) -> Option<Self::Item>;
}

/// Struct implementing the trait with associated type
struct UserRepository;

impl Repository for UserRepository {
    type Item = String;
    fn get(&self, id: u32) -> Option<Self::Item> {
        Some(format!("User {}", id))
    }
}

/// Struct with derive macros
#[derive(Debug, Clone, PartialEq, Eq)]
struct Config {
    name: String,
    enabled: bool,
}

/// Multiple derive attributes
#[derive(Debug, Clone, Copy)]
struct Point {
    x: f64,
    y: f64,
}

fn main() {
    // Create a counter using with_label
    let mut counter = Counter::with_label(10, "main");

    // Use methods
    counter.increment();
    let count = counter.get_count();
    println!("Count: {}", count);

    // Call process_value
    let result = process_value(5);

    // Use init_vec macro
    let vec = init_vec![1, 2, 3];

    // Pattern matching on enum
    let color = Color::Custom(255, 0, 128);
    match color {
        Color::Red => println!("Red"),
        Color::Green => println!("Green"),
        Color::Blue => println!("Blue"),
        Color::Custom(r, g, b) => println!("Custom: RGB({}, {}, {})", r, g, b),
    }

    // Use generic function
    let sum = add(5, 10);

    // Use Container
    let container = Container { value: 42 };
    let val = container.get();

    // Access inner module function
    let inner_msg = inner_function();
}