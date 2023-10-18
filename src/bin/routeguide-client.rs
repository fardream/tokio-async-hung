use std::error::Error;

use rand::rngs::ThreadRng;
use rand::Rng;
use tonic::transport::Channel;
use tonic::Request;

use routeguide::route_guide_client::RouteGuideClient;
use routeguide::Point;

pub mod routeguide {
    tonic::include_proto!("routeguide");
}

async fn run_record_route(mut client: RouteGuideClient<Channel>) -> Result<(), Box<dyn Error>> {
    let mut rng = rand::thread_rng();
    let point_count: i32 = 20;

    let mut points = vec![];
    for _ in 0..=point_count {
        points.push(random_point(&mut rng))
    }

    let (sender, receiver) = tokio::sync::mpsc::channel(90);
    println!("Traversing {} points", points.len());

    let t = tokio::spawn(async move {
        let request = Request::new(tokio_stream::wrappers::ReceiverStream::new(receiver));
        match client.record_route(request).await {
            Ok(response) => println!("SUMMARY: {:?}", response.into_inner()),
            Err(e) => println!("something went wrong: {:?}", e),
        }
    });

    for p in points.into_iter().take(5) {
        sender.send(p).await.unwrap();
    }

    tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
    panic!("what");

    drop(sender);

    t.await?;

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = RouteGuideClient::connect("http://[::1]:10000")
        .await?
        .max_decoding_message_size(128 * 1024 * 1024)
        .max_encoding_message_size(128 * 1024 * 1024);

    println!("\n*** CLIENT STREAMING ***");
    run_record_route(client).await?;

    Ok(())
}

fn random_point(rng: &mut ThreadRng) -> Point {
    let latitude = (rng.gen_range(0..180) - 90) * 10_000_000;
    let longitude = (rng.gen_range(0..360) - 180) * 10_000_000;
    Point {
        latitude,
        longitude,
    }
}
