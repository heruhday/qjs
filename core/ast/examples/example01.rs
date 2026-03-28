use ::ast::*;

fn main() {
    let program =
        parse("console.log('hello'); for (let x of [1, 2, 3]) { console.log(x); }").unwrap();

    println!("{:#?}", program);
}

// Command line: cargo run --package ast --example example01 --release > .\out\output.txt 2>&1
