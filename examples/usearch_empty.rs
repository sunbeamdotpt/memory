use usearch::{new_index, IndexOptions, MetricKind, ScalarKind};

fn main() {
    let mut options = IndexOptions::default();
    options.dimensions = 3;
    options.metric = MetricKind::Cos;
    options.quantization = ScalarKind::F32;
    
    let index = new_index(&options).unwrap();
    index.reserve(10).unwrap();
    
    index.add(1, &[1.0, 0.0, 0.0]).unwrap();
    index.add(2, &[0.0, 1.0, 0.0]).unwrap();
    
    println!("Size: {}", index.size());
    
    let path = "/tmp/usearch_test.index";
    match index.save(path) {
        Ok(_) => {
            println!("Saved to file");
            let data = std::fs::read(path).unwrap();
            println!("File size: {} bytes", data.len());
            
            // Try loading from file
            let index2 = new_index(&options).unwrap();
            index2.load(path).unwrap();
            let results = index2.search(&[1.0, 0.0, 0.0], 2).unwrap();
            println!("Loaded and searched: keys={:?}, distances={:?}", results.keys, results.distances);
        }
        Err(e) => println!("Error saving: {}", e),
    }
}
