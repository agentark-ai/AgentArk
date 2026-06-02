//! Document chunk entity for RAG

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "document_chunks")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub document_id: String,
    pub chunk_index: i32,
    pub content: String,
    #[sea_orm(column_type = "Vector(Some(super::PGVECTOR_EMBEDDING_DIM))")]
    pub embedding: Option<PgVector>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::document::Entity",
        from = "Column::DocumentId",
        to = "super::document::Column::Id",
        on_update = "Cascade",
        on_delete = "Cascade"
    )]
    Document,
}

impl Related<super::document::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Document.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
