use k8s_openapi::api::core::v1::LocalObjectReference;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::AsRefStr;

#[cfg(doc)]
use crate::labels::get_recommended_labels;

pub const STACKABLE_DOCKER_REPO: &str = "docker.stackable.tech/stackable";

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductImage {
    #[serde(flatten)]
    image_selection: ProductImageSelection,

    #[serde(default)]
    /// [Pull policy](https://kubernetes.io/docs/concepts/containers/images/#image-pull-policy) used when pulling the Images
    pull_policy: PullPolicy,

    /// [Image pull secrets](https://kubernetes.io/docs/concepts/containers/images/#specifying-imagepullsecrets-on-a-pod) to pull images from a private registry
    pull_secrets: Option<Vec<LocalObjectReference>>,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[serde(untagged)]
pub enum ProductImageSelection {
    // Order matters!
    // The variants will be tried from top to bottom
    Custom(ProductImageCustom),
    StackableVersion(ProductImageStackableVersion),
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductImageCustom {
    /// Overwrite the docker image.
    /// Specify the full docker image name, e.g. `docker.stackable.tech/stackable/superset:1.4.1-stackable2.1.0`
    custom: String,
    /// Version of the product, e.g. `1.4.1`.
    product_version: String,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductImageStackableVersion {
    /// Version of the product, e.g. `1.4.1`.
    product_version: String,
    /// Stackable version of the product, e.g. 2.1.0
    stackable_version: String,
    /// Name of the docker repo, e.g. `docker.stackable.tech/stackable`
    repo: Option<String>,
}

#[derive(Clone, Debug, PartialEq, JsonSchema)]
pub struct ResolvedProductImage {
    /// Version of the product, e.g. `1.4.1`.
    pub product_version: String,
    /// App version as formatted for [`get_recommended_labels`]
    pub app_version_label: String,
    /// Image to be used for the product image e.g. `docker.stackable.tech/stackable/superset:1.4.1-stackable2.1.0`
    pub image: String,
    /// Image pull policy for the containers using the product image
    pub image_pull_policy: String,
    /// Image pull secrets for the containers using the product image
    pub pull_secrets: Option<Vec<LocalObjectReference>>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename = "PascalCase")]
#[derive(AsRefStr)]
pub enum PullPolicy {
    #[default]
    IfNotPresent,
    Always,
    Never,
}

impl ProductImage {
    pub fn resolve(&self, image_base_name: &str) -> ResolvedProductImage {
        let image_pull_policy = self.pull_policy.as_ref().to_string();
        let pull_secrets = self.pull_secrets.clone();

        match &self.image_selection {
            ProductImageSelection::Custom(custom) => {
                let custom_image_tag = custom
                    .custom
                    .split_once(':')
                    .map_or("latest", |splits| splits.1);
                let app_version_label = format!("{}-{}", custom.product_version, custom_image_tag);
                ResolvedProductImage {
                    product_version: custom.product_version.to_string(),
                    app_version_label,
                    image: custom.custom.to_string(),
                    image_pull_policy,
                    pull_secrets,
                }
            }
            ProductImageSelection::StackableVersion(stackable_version) => {
                let repo = stackable_version
                    .repo
                    .as_deref()
                    .unwrap_or(STACKABLE_DOCKER_REPO);
                let image = format!(
                    "{repo}/{image_base_name}:{product_version}-stackable{stackable_version}",
                    product_version = stackable_version.product_version,
                    stackable_version = stackable_version.stackable_version,
                );
                let app_version_label = format!(
                    "{product_version}-stackable{stackable_version}",
                    product_version = stackable_version.product_version,
                    stackable_version = stackable_version.stackable_version,
                );
                ResolvedProductImage {
                    product_version: stackable_version.product_version.to_string(),
                    app_version_label,
                    image,
                    image_pull_policy,
                    pull_secrets,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use rstest::rstest;

    #[rstest]
    #[case::stackable_version_without_repo(
        "superset",
        r#"
        productVersion: 1.4.1
        stackableVersion: 2.1.0
        "#,
        ResolvedProductImage {
            image: "docker.stackable.tech/stackable/superset:1.4.1-stackable2.1.0".to_string(),
            app_version_label: "1.4.1-stackable2.1.0".to_string(),
            product_version: "1.4.1".to_string(),
            image_pull_policy: "IfNotPresent".to_string(),
            pull_secrets: None,
        }
    )]
    #[case::stackable_version_with_repo(
        "trino",
        r#"
        productVersion: 1.4.1
        stackableVersion: 2.1.0
        repo: my.corp/myteam/stackable
        "#,
        ResolvedProductImage {
            image: "my.corp/myteam/stackable/trino:1.4.1-stackable2.1.0".to_string(),
            app_version_label: "1.4.1-stackable2.1.0".to_string(),
            product_version: "1.4.1".to_string(),
            image_pull_policy: "IfNotPresent".to_string(),
            pull_secrets: None,
        }
    )]
    #[case::custom_without_tag(
        "superset",
        r#"
        custom: my.corp/myteam/stackable/superset
        productVersion: 1.4.1
        "#,
        ResolvedProductImage {
            image: "my.corp/myteam/stackable/superset".to_string(),
            app_version_label: "1.4.1-latest".to_string(),
            product_version: "1.4.1".to_string(),
            image_pull_policy: "IfNotPresent".to_string(),
            pull_secrets: None,
        }
    )]
    #[case::custom_with_tag(
        "superset",
        r#"
        custom: my.corp/myteam/stackable/superset:latest-and-greatest
        productVersion: 1.4.1
        "#,
        ResolvedProductImage {
            image: "my.corp/myteam/stackable/superset:latest-and-greatest".to_string(),
            app_version_label: "1.4.1-latest-and-greatest".to_string(),
            product_version: "1.4.1".to_string(),
            image_pull_policy: "IfNotPresent".to_string(),
            pull_secrets: None,
        }
    )]
    #[case::custom_takes_precedence(
        "superset",
        r#"
        custom: my.corp/myteam/stackable/superset:latest-and-greatest
        productVersion: 1.4.1
        stackableVersion: not-used
        "#,
        ResolvedProductImage {
            image: "my.corp/myteam/stackable/superset:latest-and-greatest".to_string(),
            app_version_label: "1.4.1-latest-and-greatest".to_string(),
            product_version: "1.4.1".to_string(),
            image_pull_policy: "IfNotPresent".to_string(),
            pull_secrets: None,
        }
    )]
    #[case::pull_policy_if_not_present(
        "superset",
        r#"
        custom: my.corp/myteam/stackable/superset:latest-and-greatest
        productVersion: 1.4.1
        pullPolicy: IfNotPresent
        "#,
        ResolvedProductImage {
            image: "my.corp/myteam/stackable/superset:latest-and-greatest".to_string(),
            app_version_label: "1.4.1-latest-and-greatest".to_string(),
            product_version: "1.4.1".to_string(),
            image_pull_policy: "IfNotPresent".to_string(),
            pull_secrets: None,
        }
    )]
    #[case::pull_policy_always(
        "superset",
        r#"
        custom: my.corp/myteam/stackable/superset:latest-and-greatest
        productVersion: 1.4.1
        pullPolicy: Always
        "#,
        ResolvedProductImage {
            image: "my.corp/myteam/stackable/superset:latest-and-greatest".to_string(),
            app_version_label: "1.4.1-latest-and-greatest".to_string(),
            product_version: "1.4.1".to_string(),
            image_pull_policy: "Always".to_string(),
            pull_secrets: None,
        }
    )]
    #[case::pull_policy_never(
        "superset",
        r#"
        custom: my.corp/myteam/stackable/superset:latest-and-greatest
        productVersion: 1.4.1
        pullPolicy: Never
        "#,
        ResolvedProductImage {
            image: "my.corp/myteam/stackable/superset:latest-and-greatest".to_string(),
            app_version_label: "1.4.1-latest-and-greatest".to_string(),
            product_version: "1.4.1".to_string(),
            image_pull_policy: "Never".to_string(),
            pull_secrets: None,
        }
    )]
    #[case::pull_secrets(
        "superset",
        r#"
        custom: my.corp/myteam/stackable/superset:latest-and-greatest
        productVersion: 1.4.1
        pullPolicy: Always
        pullSecrets:
        - name: myPullSecrets1
        - name: myPullSecrets2
        "#,
        ResolvedProductImage {
            image: "my.corp/myteam/stackable/superset:latest-and-greatest".to_string(),
            app_version_label: "1.4.1-latest-and-greatest".to_string(),
            product_version: "1.4.1".to_string(),
            image_pull_policy: "Always".to_string(),
            pull_secrets: Some(vec![LocalObjectReference{name: Some("myPullSecrets1".to_string())}, LocalObjectReference{name: Some("myPullSecrets2".to_string())}]),
        }
    )]
    fn test_correct_resolved_image(
        #[case] image_base_name: String,
        #[case] input: String,
        #[case] expected: ResolvedProductImage,
    ) {
        let product_image: ProductImage = serde_yaml::from_str(&input).expect("Illegal test input");
        let resolved_product_image = product_image.resolve(&image_base_name);

        assert_eq!(resolved_product_image, expected);
    }

    #[rstest]
    #[case::custom(
        r#"
        custom: my.corp/myteam/stackable/superset:latest-and-greatest
        "#,
        "data did not match any variant of untagged enum ProductImageSelection at line 2 column 9"
    )]
    #[case::product_version(
        r#"
        productVersion: 1.4.1
        "#,
        "data did not match any variant of untagged enum ProductImageSelection at line 2 column 9"
    )]
    #[case::stackable_version(
        r#"
        stackableVersion: 2.1.0
        "#,
        "data did not match any variant of untagged enum ProductImageSelection at line 2 column 9"
    )]
    #[case::empty(
        "{}",
        "data did not match any variant of untagged enum ProductImageSelection"
    )]
    fn test_invalid_image(#[case] input: String, #[case] expected: String) {
        let err = serde_yaml::from_str::<ProductImage>(&input).expect_err("Must be error");

        assert_eq!(err.to_string(), expected);
    }
}
