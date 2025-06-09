use crate::config::Config;
use anyhow::Result;
use std::sync::Arc;
use surrealdb::{
    engine::remote::ws::{Client, Ws},
    opt::auth::Root,
    Surreal,
};

pub type Pool = Arc<Surreal<Client>>;

pub async fn create_pool(config: Arc<Config>) -> Result<Pool> {
    let db = Surreal::new::<Ws>(&config.db_url).await?;

    db.signin(Root {
        username: &config.db_user,
        password: &config.db_pass,
    })
    .await?;

    db.use_ns(&config.db_namespace)
        .use_db(&config.db_database)
        .await?;

    Ok(Arc::new(db))
}