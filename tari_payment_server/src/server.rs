use std::{net::IpAddr, time::Duration};

use actix_jwt_auth_middleware::use_jwt::UseJWTOnApp;
use actix_web::{
    dev::{Server, Service, ServiceRequest},
    http::KeepAlive,
    middleware::Logger,
    web,
    App,
    HttpServer,
};
use futures::{future::ok, FutureExt};
use log::*;
use shopify_tools::ShopifyApi;
use tari_payment_engine::{
    events::EventProducers,
    tpe_api::{exchange_rate_api::ExchangeRateApi, wallet_api::WalletManagementApi},
    AccountApi,
    AuthApi,
    OrderFlowApi,
    SqliteDatabase,
    WalletAuthApi,
};

use crate::{
    auth::{build_tps_authority, TokenIssuer},
    config::{ServerConfig, ServerOptions},
    errors::{AuthError, ServerError, ServerError::AuthenticationError},
    expiry_worker::start_expiry_worker,
    helpers::get_remote_ip,
    integrations::shopify::create_shopify_event_handlers,
    middleware::HmacMiddlewareFactory,
    routes::{
        health,
        AddAuthorizedWalletRoute,
        AddressesRoute,
        AuthRoute,
        BalanceRoute,
        CancelOrderRoute,
        CheckTokenRoute,
        ClaimOrderRoute,
        CreditorsRoute,
        CustomerIdsRoute,
        FulfilOrderRoute,
        GetAuthorizedAddressesRoute,
        GetAuthorizedWalletsRoute,
        GetExchangeRateRoute,
        HistoryForAddressRoute,
        HistoryForCustomerRoute,
        IncomingPaymentNotificationRoute,
        IssueCreditRoute,
        MyBalanceRoute,
        MyHistoryRoute,
        MyOrdersRoute,
        MyPaymentsRoute,
        MyUnfulfilledOrdersRoute,
        OrderByIdRoute,
        OrdersRoute,
        OrdersSearchRoute,
        PaymentForOrderRoute,
        PaymentsRoute,
        ReassignOrderRoute,
        RemoveAuthorizedWalletRoute,
        RescanOpenOrdersRoute,
        ResetOrderRoute,
        SettleAddressRoute,
        SettleCustomerRoute,
        SettleMyAccountRoute,
        TxConfirmationNotificationRoute,
        UnfulfilledOrdersRoute,
        UpdateOrderMemoRoute,
        UpdatePriceRoute,
        UpdateRolesRoute,
    },
    shopify_routes::{ShopifyOnProductUpdatedRoute, ShopifyWebhookRoute, UpdateShopifyExchangeRateRoute},
};

/// Defines the log format for the access log middleware.
const LOG_FORMAT: &str = concat!(
    "%t ",                                   // Time when the request was started to process
    "%a ",                                   // Remote IP-address (IP-address of proxy if using reverse proxy)
    "x-forwarded-for: %{X-Forwarded-For}i ", // X-Forwarded-For header
    "forwarded: %{Forwarded}i ",             // Forwarded header
    "\"%r\" ",                               // First line of request
    "%s ",                                   // Response status code
    "ua:\"%{User-Agent}i\" ",                // User agent
    "%D ms",                                 // Time taken to serve the request in milliseconds
);

pub async fn run_server(config: ServerConfig) -> Result<(), ServerError> {
    let db = SqliteDatabase::new_with_url(&config.database_url, 25)
        .await
        .map_err(|e| ServerError::InitializeError(e.to_string()))?;
    // Shopify is the only supported integration at the moment. In future, this would be conditional code based on a
    // configuration file.
    info!("🚦️ Configuring Shopify event handlers...");
    let shopify_config = config.shopify_config.shopify_api_config();
    let shopify_handlers = create_shopify_event_handlers(shopify_config)
        .map_err(|e| ServerError::InitializeError(format!("Failed to create Shopify event handlers: {e}")))?;
    let producers = shopify_handlers.producers();
    let srv = create_server_instance(config.clone(), db.clone(), producers.clone())?;
    // Start the event handlers
    tokio::spawn(async move {
        info!("🚦️ Starting shopify event handlers...");
        shopify_handlers.start_handlers().await;
    });
    let _never_ends =
        start_expiry_worker(db.clone(), producers.clone(), config.unclaimed_order_timeout, config.unpaid_order_timeout);
    srv.await.map_err(|e| ServerError::Unspecified(e.to_string()))
}

#[allow(clippy::too_many_lines)]
pub fn create_server_instance(
    config: ServerConfig,
    db: SqliteDatabase,
    producers: EventProducers,
) -> Result<Server, ServerError> {
    let proxy_config = ServerOptions::from_config(&config);
    let shopify_config = config.shopify_config.shopify_api_config();
    let order_id_field = config.shopify_config.order_id_field;
    let shopify_api = ShopifyApi::new(shopify_config).map_err(|e| {
        let msg = format!("Failed to create Shopify API: {e}");
        error!("{msg}");
        ServerError::InitializeError(msg)
    })?;
    let srv = HttpServer::new(move || {
        let orders_api = OrderFlowApi::new(db.clone(), producers.clone());
        let auth_api = AuthApi::new(db.clone());
        let jwt_signer = TokenIssuer::new(&config.auth);
        let authority = build_tps_authority(config.auth.clone());
        let accounts_api = AccountApi::new(db.clone());
        let wallet_auth = WalletAuthApi::new(db.clone());
        let wallet_manager = WalletManagementApi::new(db.clone());
        let exchange_rates = ExchangeRateApi::new(db.clone());
        let hmac_middleware = HmacMiddlewareFactory::new(
            "X-Shopify-Hmac-Sha256",
            config.shopify_config.hmac_secret.clone(),
            config.shopify_config.hmac_checks,
        );

        let mut app = App::new()
            .wrap(Logger::new(LOG_FORMAT).log_target("access_log"))
            .app_data(web::Data::new(orders_api))
            .app_data(web::Data::new(accounts_api))
            .app_data(web::Data::new(shopify_api.clone()))
            .app_data(web::Data::new(auth_api))
            .app_data(web::Data::new(jwt_signer))
            .app_data(web::Data::new(wallet_auth))
            .app_data(web::Data::new(wallet_manager))
            .app_data(web::Data::new(exchange_rates))
            .app_data(web::Data::new(proxy_config))
            .app_data(web::Data::new(order_id_field));
        // Routes that require authentication
        let auth_scope = web::scope("/api")
            .service(UpdateRolesRoute::<SqliteDatabase>::new())
            .service(BalanceRoute::<SqliteDatabase>::new())
            .service(MyBalanceRoute::<SqliteDatabase>::new())
            .service(MyHistoryRoute::<SqliteDatabase>::new())
            .service(HistoryForAddressRoute::<SqliteDatabase>::new())
            .service(HistoryForCustomerRoute::<SqliteDatabase>::new())
            .service(MyOrdersRoute::<SqliteDatabase>::new())
            .service(MyUnfulfilledOrdersRoute::<SqliteDatabase>::new())
            .service(UnfulfilledOrdersRoute::<SqliteDatabase>::new())
            .service(OrdersRoute::<SqliteDatabase>::new())
            .service(OrderByIdRoute::<SqliteDatabase>::new())
            .service(MyPaymentsRoute::<SqliteDatabase>::new())
            .service(PaymentsRoute::<SqliteDatabase>::new())
            .service(PaymentForOrderRoute::<SqliteDatabase>::new())
            .service(OrdersSearchRoute::<SqliteDatabase>::new())
            .service(CreditorsRoute::<SqliteDatabase>::new())
            .service(IssueCreditRoute::<SqliteDatabase>::new())
            .service(FulfilOrderRoute::<SqliteDatabase>::new())
            .service(CancelOrderRoute::<SqliteDatabase>::new())
            .service(UpdateOrderMemoRoute::<SqliteDatabase>::new())
            .service(UpdatePriceRoute::<SqliteDatabase>::new())
            .service(ReassignOrderRoute::<SqliteDatabase>::new())
            .service(ResetOrderRoute::<SqliteDatabase>::new())
            .service(GetExchangeRateRoute::<SqliteDatabase>::new())
            .service(UpdateShopifyExchangeRateRoute::<SqliteDatabase>::new())
            .service(CustomerIdsRoute::<SqliteDatabase>::new())
            .service(AddressesRoute::<SqliteDatabase>::new())
            .service(GetAuthorizedWalletsRoute::<SqliteDatabase>::new())
            .service(RemoveAuthorizedWalletRoute::<SqliteDatabase>::new())
            .service(AddAuthorizedWalletRoute::<SqliteDatabase>::new())
            .service(SettleAddressRoute::<SqliteDatabase>::new())
            .service(SettleCustomerRoute::<SqliteDatabase>::new())
            .service(SettleMyAccountRoute::<SqliteDatabase>::new())
            .service(RescanOpenOrdersRoute::<SqliteDatabase, SqliteDatabase>::new())
            .service(CheckTokenRoute::new());
        let use_x_forwarded_for = config.use_x_forwarded_for;
        let use_forwarded = config.use_forwarded;
        let shopify_whitelist = config.shopify_config.whitelist.clone();
        let shopify_scope = web::scope("/shopify")
            .wrap_fn(move |req, srv| {
                let whitelisted = is_whitelisted(use_x_forwarded_for, use_forwarded, &shopify_whitelist, &req);
                if whitelisted {
                    srv.call(req)
                } else {
                    ok(req.error_response(AuthenticationError(AuthError::ForbiddenPeer))).boxed_local()
                }
            })
            .wrap(hmac_middleware)
            .service(ShopifyWebhookRoute::<SqliteDatabase, SqliteDatabase>::new())
            .service(ShopifyOnProductUpdatedRoute::<SqliteDatabase>::new())
            .service(health);
        let wallet_scope = web::scope("/wallet")
            .service(GetAuthorizedAddressesRoute::<SqliteDatabase>::new())
            .service(IncomingPaymentNotificationRoute::<SqliteDatabase, SqliteDatabase>::new())
            .service(TxConfirmationNotificationRoute::<SqliteDatabase, SqliteDatabase>::new());
        app = app.service(wallet_scope);
        app.use_jwt(authority.clone(), auth_scope)
            .service(health)
            .service(AuthRoute::<SqliteDatabase>::new())
            .service(ClaimOrderRoute::<SqliteDatabase>::new())
            .service(shopify_scope)
    })
    .keep_alive(KeepAlive::Timeout(Duration::from_secs(600)))
    .bind((config.host.as_str(), config.port))?
    .run();
    Ok(srv)
}

fn is_whitelisted(
    use_x_forwarded_for: bool,
    use_forwarded: bool,
    shopify_whitelist: &Option<Vec<IpAddr>>,
    req: &ServiceRequest,
) -> bool {
    let peer_ip = get_remote_ip(req.request(), use_x_forwarded_for, use_forwarded);
    match (peer_ip, &shopify_whitelist) {
        (Some(ip), Some(whitelist)) => {
            let result = whitelist.contains(&ip);
            info!("Shopify webhook request from {ip}. Permitted peer: {result}");
            result
        },
        (_, None) => true,
        (None, Some(_)) => {
            warn!("No IP address found in shopify remote peer request. denying access.");
            false
        },
    }
}
