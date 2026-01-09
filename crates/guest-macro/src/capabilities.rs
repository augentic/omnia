use proc_macro2::TokenStream;
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{Ident, Result, Token};

pub struct Capabilities {
    pub capabilities: Vec<Capability>,
}

impl Parse for Capabilities {
    fn parse(input: ParseStream) -> Result<Self> {
        let capabilities = Punctuated::<Capability, Token![,]>::parse_terminated(input)?;
        Ok(Self {
            capabilities: capabilities.into_iter().collect(),
        })
    }
}

pub struct Capability {
    pub name: Ident,
}

impl Parse for Capability {
    fn parse(input: ParseStream) -> Result<Self> {
        let name: Ident = input.parse()?;
        Ok(Self { name })
    }
}

pub fn expand(capabilities: &Capabilities) -> TokenStream {
    let impls = capabilities.capabilities.iter().map(expand_capability);

    quote! {
        mod capabilities {
            use std::any::Any;
            use std::error::Error;

            use warp_sdk::anyhow::{Context, Result};
            use warp_sdk::bytes::Bytes;
            use warp_sdk::http::{Request, Response};
            use warp_sdk::{wasi_http, wasi_identity, wasi_keyvalue, wasi_messaging};
            use warp_sdk::{Config, HttpRequest, Identity, Message, Publisher, StateStore};

            use super::environment::ConfigSettings;
            use super::*;

            #[derive(Clone, Default)]
            pub struct Provider {
                pub config: ConfigSettings,
            }

            impl Provider {
                pub fn new() -> Self {
                    Self::default()
                }
            }

            impl Config for Provider {
                async fn get(&self, key: &str) -> Result<String> {
                    <ConfigSettings as Config>::get(&self.config, key).await
                }
            }

            #(#impls)*
        }
    }
}

fn expand_capability(capability: &Capability) -> TokenStream {
    let name = capability.name.to_string();

    match name.as_str() {
        "HttpRequest" => expand_http_request(),
        "Identity" => expand_identity(),
        "Publisher" => expand_publisher(),
        "StateStore" => expand_state_store(),
        _ => {
            let name_ident = &capability.name;
            quote! {
                compile_error!(concat!("unknown capability: ", stringify!(#name_ident)));
            }
        }
    }
}

fn expand_http_request() -> TokenStream {
    quote! {
        impl HttpRequest for Provider {
            async fn fetch<T>(&self, request: Request<T>) -> Result<Response<Bytes>>
            where
                T: warp_sdk::http_body::Body + Any + Send,
                T::Data: Into<Vec<u8>>,
                T::Error: Into<Box<dyn Error + Send + Sync + 'static>>,
            {
                tracing::debug!("request: {:?}", request.uri());
                wasi_http::handle(request).await
            }
        }
    }
}

fn expand_identity() -> TokenStream {
    quote! {
        impl Identity for Provider {
            async fn access_token(&self) -> Result<String> {
                use warp_sdk::wit_bindgen::block_on;

                let identity = <Self as Config>::get(&self, "AZURE_IDENTITY").await?;
                let identity = block_on(wasi_identity::credentials::get_identity(identity))?;
                let access_token = block_on(async move { identity.get_token(vec![]).await })?;
                Ok(access_token.token)
            }
        }
    }
}

fn expand_publisher() -> TokenStream {
    quote! {
        impl Publisher for Provider {
            async fn send(&self, topic: &str, message: &Message) -> Result<()> {
                use wasi_messaging::producer;
                use wasi_messaging::types::Client;

                tracing::debug!("sending to topic: {topic}");

                let client = Client::connect("kafka".to_string())
                    .await
                    .context("connecting to broker")?;
                let msg = wasi_messaging::types::Message::new(&message.payload);
                let env = <Self as Config>::get(&self, "ENV").await.unwrap_or_default();
                let topic = format!("{env}-{topic}");

                if let Err(e) = producer::send(&client, topic.clone(), msg).await {
                    tracing::error!(
                        monotonic_counter.publishing_errors = 1,
                        error = %e,
                        topic = %topic,
                    );
                } else {
                    tracing::info!(
                        monotonic_counter.messages_sent = 1,
                        topic = %topic,
                    );
                }

                Ok(())
            }
        }
    }
}

fn expand_state_store() -> TokenStream {
    quote! {
        impl StateStore for Provider {
            async fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
                let bucket = wasi_keyvalue::cache::open("cache")
                    .await
                    .context("opening cache")?;
                bucket.get(key).await.context("reading state from cache")
            }

            async fn set(
                &self,
                key: &str,
                value: &[u8],
                ttl_secs: Option<u64>,
            ) -> Result<Option<Vec<u8>>> {
                let bucket = wasi_keyvalue::cache::open("cache")
                    .await
                    .context("opening cache")?;
                bucket
                    .set(key, value, ttl_secs)
                    .await
                    .context("writing state to cache")
            }

            async fn delete(&self, key: &str) -> Result<()> {
                let bucket = wasi_keyvalue::cache::open("cache")
                    .await
                    .context("opening cache")?;
                bucket.delete(key).await.context("deleting state from cache")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use quote::quote;

    use super::*;

    #[test]
    fn parse_capabilities() {
        let input = quote! {
            HttpRequest,
            Identity,
            Publisher,
            StateStore
        };

        let parsed: Capabilities = syn::parse2(input).expect("should parse");
        assert_eq!(parsed.capabilities.len(), 4);

        assert_eq!(parsed.capabilities[0].name.to_string(), "HttpRequest");
        assert_eq!(parsed.capabilities[1].name.to_string(), "Identity");
        assert_eq!(parsed.capabilities[2].name.to_string(), "Publisher");
        assert_eq!(parsed.capabilities[3].name.to_string(), "StateStore");
    }

    #[test]
    fn parse_single_capability() {
        let input = quote! {
            HttpRequest
        };

        let parsed: Capabilities = syn::parse2(input).expect("should parse");
        assert_eq!(parsed.capabilities.len(), 1);
        assert_eq!(parsed.capabilities[0].name.to_string(), "HttpRequest");
    }
}
