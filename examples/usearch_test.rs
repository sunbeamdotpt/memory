use usearch::{new_index, IndexOptions, MetricKind, ScalarKind};

fn main() {
    let mut options = IndexOptions::default();
    options.dimensions = 3;
    options.metric = MetricKind::Cos;
    options.quantization = ScalarKind::F32;
    
    let index = new_index(&options).unwrap();
    index.reserve(10).unwrap();
    
    index.add(1, &[1.0_f32, 0.0, 0.0]).unwrap();
    index.add(2, &[0.0_f32, 1.0, 0.0]).unwrap();
    index.add(3, &[0.0_f32, 0.0, 1.0]).unwrap();
    
    let results = index.search(&[1.0_f32, 0.0, 0.0], 2).unwrap();
    println!("Search results: keys={:?}, distances={:?}", results.keys, results.distances);
    
    // Save to buffer
    let mut buf = Vec::new();
    index.save_to_buffer(&mut buf).unwrap();
    println!("Saved {} bytes", buf.len());
    
    // Load from buffer
    let index2 = new_index(&options).unwrap();
    index2.load_from_buffer(&buf).unwrap();
    
    let results2 = index2.search(&[1.0_f32, 0.0, 0.0], 2).unwrap();
    println!("Loaded search results: keys={:?}, distances={:?}", results2.keys, results2.distances);
    
    // Remove
    index.remove(1).unwrap();
    let results3 = index.search(&[1.0_f32, 0.0, 0.0], 2).unwrap();
    println!("After remove: keys={:?}", results3.keys);
}
