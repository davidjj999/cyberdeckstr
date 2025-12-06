use textwrap::wrap;

fn main() {
    let text = "Hello\rWorld";
    let wrapped = wrap(text, 20);
    println!("Input: {:?}", text);
    for (i, line) in wrapped.iter().enumerate() {
        println!("Line {}: {:?} (len: {})", i, line, line.len());
    }

    let text2 = "Line1\nLine2";
    let wrapped2 = wrap(text2, 20); // Width 20
    println!("Input: {:?}", text2);
    for (i, line) in wrapped2.iter().enumerate() {
        println!("Line {}: {:?} (len: {})", i, line, line.len());
    }
}
