mod cucumber;
mod support;

use crate::cucumber::ShopifyWorld;
use ::cucumber::codegen::LocalBoxFuture;
use ::cucumber::event::ScenarioFinished;

use ::cucumber::{gherkin, writer, World};
use futures_util::FutureExt;
use log::*;
use tari_payment_engine::PaymentGatewayDatabase;
use tokio::runtime::Runtime;

fn main() {
    dotenvy::from_filename(".env.test").ok();
    env_logger::init();
    let sys = Runtime::new().unwrap();
    sys.block_on(
        ShopifyWorld::cucumber()
            .with_writer(writer::Libtest::or_basic())
            .after(|_f, _r, scenario, ev, w| post_test_hook(scenario, ev, w))
            .run("tests/features"),
    );
    info!("🚀️ Tests complete");
}

fn post_test_hook<'a>(
    scenario: &'a gherkin::Scenario,
    ev: &'a ScenarioFinished,
    world: Option<&'a mut ShopifyWorld>,
) -> LocalBoxFuture<'a, ()> {
    let fut = async move {
        trace!("🚀️ After-scenario hook running for \"{}\"", scenario.name);
        if let Some(ShopifyWorld { system: Some(sys) }) = world {
            let db_path = sys.db_path.clone();
            match ev {
                ScenarioFinished::StepFailed(_, _, _) | ScenarioFinished::StepSkipped => {
                    error!("🚀️ Error in scenario, database retained: {db_path}");
                }
                ScenarioFinished::StepPassed => {
                    debug!("🚀️ Scenario complete, removing database: {db_path}");
                    match sys.api.db_mut().close().await {
                        Ok(_) => {
                            let filename = db_path.replace("sqlite://", "./");
                            std::fs::remove_file(filename).expect("Error removing test database");
                        }
                        Err(e) => {
                            error!("🚀️ Error closing database: {:?}", e);
                        }
                    }
                }
                _ => trace!("🚀️ Unhandled event: {ev:?}"),
            }
        } else {
            warn!("🚀️ World was not specified. Cannot cleanup database.");
        }
        trace!("🚀️ After-scenario hook complete");
    };
    fut.boxed_local()
}
