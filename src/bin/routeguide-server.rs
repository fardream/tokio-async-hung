use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::mpsc;
use tokio_stream::{wrappers::ReceiverStream, Stream, StreamExt};
use tonic::transport::Server;
use tonic::{Request, Response, Status};

use routeguide::route_guide_server::{RouteGuide, RouteGuideServer};
use routeguide::{Feature, Point, Rectangle, RouteNote, RouteSummary};

pub mod routeguide {
    tonic::include_proto!("routeguide");
}

mod data {
    use serde::Deserialize;
    use std::fs::File;

    #[derive(Debug, Deserialize)]
    struct Feature {
        location: Location,
        name: String,
    }

    #[derive(Debug, Deserialize)]
    struct Location {
        latitude: i32,
        longitude: i32,
    }

    #[allow(dead_code)]
    pub fn load() -> Vec<super::routeguide::Feature> {
        let data_dir = std::path::PathBuf::from_iter([std::env!("CARGO_MANIFEST_DIR"), "data"]);
        let file =
            File::open(data_dir.join("route_guide_db.json")).expect("failed to open data file");

        let decoded: Vec<Feature> =
            serde_json::from_reader(&file).expect("failed to deserialize features");

        decoded
            .into_iter()
            .map(|feature| crate::routeguide::Feature {
                name: feature.name,
                location: Some(crate::routeguide::Point {
                    longitude: feature.location.longitude,
                    latitude: feature.location.latitude,
                }),
            })
            .collect()
    }
}

#[derive(Debug)]
pub struct RouteGuideService {
    features: Arc<Vec<Feature>>,
}

#[tonic::async_trait]
impl RouteGuide for RouteGuideService {
    async fn get_feature(&self, request: Request<Point>) -> Result<Response<Feature>, Status> {
        println!("GetFeature = {:?}", request);

        for feature in &self.features[..] {
            if feature.location.as_ref() == Some(request.get_ref()) {
                return Ok(Response::new(feature.clone()));
            }
        }

        Ok(Response::new(Feature::default()))
    }

    type ListFeaturesStream = ReceiverStream<Result<Feature, Status>>;

    async fn list_features(
        &self,
        request: Request<Rectangle>,
    ) -> Result<Response<Self::ListFeaturesStream>, Status> {
        println!("ListFeatures = {:?}", request);

        let (tx, rx) = mpsc::channel(4);
        let features = self.features.clone();

        tokio::spawn(async move {
            for feature in &features[..] {
                if in_range(feature.location.as_ref().unwrap(), request.get_ref()) {
                    println!("  => send {:?}", feature);
                    tx.send(Ok(feature.clone())).await.unwrap();
                }
            }

            println!(" /// done sending");
        });

        Ok(Response::new(ReceiverStream::new(rx)))
    }

    async fn record_route(
        &self,
        request: Request<tonic::Streaming<Point>>,
    ) -> Result<Response<RouteSummary>, Status> {
        println!("RecordRoute");

        let mut stream = request.into_inner();

        let mut summary = RouteSummary::default();
        let mut last_point = None;
        let now = Instant::now();

        let mut n = 0;
        while let Some(point) = stream.next().await {
            println!("get config: {}", n);
            let config = aws_config::load_from_env().await;
            println!("done get config: {:?}", config.app_name());
            n += 1;
            if point.is_err() {
                println!("{:?}", point.unwrap_err());
                break;
            }
            let point = point.unwrap();

            println!("  ==> Point = {:?}", point);

            // Increment the point count
            summary.point_count += 1;

            // Find features
            for feature in &self.features[..] {
                if feature.location.as_ref() == Some(&point) {
                    summary.feature_count += 1;
                }
            }

            // Calculate the distance
            if let Some(ref last_point) = last_point {
                summary.distance += calc_distance(last_point, &point);
            }

            last_point = Some(point);
        }

        println!("last processing: {}", n);
        let config = aws_config::load_from_env().await;
        println!("last processing done get config: {:?}", config.app_name());

        summary.elapsed_time = now.elapsed().as_secs() as i32;

        Ok(Response::new(summary))
    }

    type RouteChatStream = Pin<Box<dyn Stream<Item = Result<RouteNote, Status>> + Send + 'static>>;

    async fn route_chat(
        &self,
        request: Request<tonic::Streaming<RouteNote>>,
    ) -> Result<Response<Self::RouteChatStream>, Status> {
        println!("RouteChat");

        let mut notes = HashMap::new();
        let mut stream = request.into_inner();

        let output = async_stream::try_stream! {
            while let Some(note) = stream.next().await {
                let note = note?;

                let location = note.location.clone().unwrap();

                let location_notes = notes.entry(location).or_insert(vec![]);
                location_notes.push(note);

                for note in location_notes {
                    yield note.clone();
                }
            }
        };

        Ok(Response::new(Box::pin(output) as Self::RouteChatStream))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = "[::1]:10000".parse().unwrap();

    println!("RouteGuideServer listening on: {}", addr);

    let route_guide = RouteGuideService {
        features: Arc::new(data::load()),
    };

    let svc = RouteGuideServer::new(route_guide)
        .max_decoding_message_size(128 * 1024 * 1024)
        .max_encoding_message_size(128 * 1024 * 1024);

    Server::builder().add_service(svc).serve(addr).await?;

    Ok(())
}

impl Eq for Point {}

fn in_range(point: &Point, rect: &Rectangle) -> bool {
    use std::cmp;

    let lo = rect.lo.as_ref().unwrap();
    let hi = rect.hi.as_ref().unwrap();

    let left = cmp::min(lo.longitude, hi.longitude);
    let right = cmp::max(lo.longitude, hi.longitude);
    let top = cmp::max(lo.latitude, hi.latitude);
    let bottom = cmp::min(lo.latitude, hi.latitude);

    point.longitude >= left
        && point.longitude <= right
        && point.latitude >= bottom
        && point.latitude <= top
}

/// Calculates the distance between two points using the "haversine" formula.
/// This code was taken from http://www.movable-type.co.uk/scripts/latlong.html.
fn calc_distance(p1: &Point, p2: &Point) -> i32 {
    const CORD_FACTOR: f64 = 1e7;
    const R: f64 = 6_371_000.0; // meters

    let lat1 = p1.latitude as f64 / CORD_FACTOR;
    let lat2 = p2.latitude as f64 / CORD_FACTOR;
    let lng1 = p1.longitude as f64 / CORD_FACTOR;
    let lng2 = p2.longitude as f64 / CORD_FACTOR;

    let lat_rad1 = lat1.to_radians();
    let lat_rad2 = lat2.to_radians();

    let delta_lat = (lat2 - lat1).to_radians();
    let delta_lng = (lng2 - lng1).to_radians();

    let a = (delta_lat / 2f64).sin() * (delta_lat / 2f64).sin()
        + (lat_rad1).cos() * (lat_rad2).cos() * (delta_lng / 2f64).sin() * (delta_lng / 2f64).sin();

    let c = 2f64 * a.sqrt().atan2((1f64 - a).sqrt());

    (R * c) as i32
}
