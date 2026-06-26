use proc_macro2::TokenStream;
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{Ident, LitStr, Path, Result, Token};

use crate::guest::{Config, handler_name};

pub struct Messaging {
    pub topics: Vec<Topic>,
}

impl Parse for Messaging {
    fn parse(input: ParseStream) -> Result<Self> {
        let topics = Punctuated::<Topic, Token![,]>::parse_terminated(input)?;
        Ok(Self {
            topics: topics.into_iter().collect(),
        })
    }
}

pub struct Topic {
    pub pattern: LitStr,
    pub message: Path,
    pub handler: Ident,
}

impl Parse for Topic {
    fn parse(input: ParseStream) -> Result<Self> {
        let pattern: LitStr = input.parse()?;
        input.parse::<Token![:]>()?;

        let message: Path = input.parse()?;
        let handler = handler_name(&pattern);

        Ok(Self {
            pattern,
            message,
            handler,
        })
    }
}

pub fn expand(messaging: &Messaging, config: &Config) -> TokenStream {
    let topic_arms = messaging.topics.iter().map(expand_topic);
    let processors = messaging.topics.iter().map(|t| expand_handler(t, config));

    quote! {
        mod messaging {
            use omnia_sdk::omnia_wasi_messaging::types::{Error, Message};
            use omnia_sdk::{wasi_messaging, omnia_wasi_otel};
            use omnia_sdk::Handler;

            use super::*;

            pub struct Messaging;
            omnia_wasi_messaging::export!(Messaging with_types_in omnia_wasi_messaging);

            // Message handler
            impl omnia_wasi_messaging::incoming_handler::Guest for Messaging {
                #[omnia_wasi_otel::instrument]
                async fn handle(message: Message) -> Result<(), Error> {
                    let topic = message
                        .topic()
                        .ok_or_else(|| Error::Other("message is missing topic".to_string()))?;

                    let result = match topic.as_str() {
                        #(#topic_arms)*
                        _ => return Err(Error::Other(format!("unhandled topic: {topic}"))),
                    };

                    result.map_err(|e| Error::Other(e.to_string()))
                }
            }

            // Message processors
            #(#processors)*
        }
    }
}

fn expand_topic(topic: &Topic) -> TokenStream {
    let pattern = &topic.pattern;
    let handler = &topic.handler;

    quote! {
        t if t.contains(#pattern) => #handler(message.data()).await,
    }
}

fn expand_handler(topic: &Topic, config: &Config) -> TokenStream {
    let handler_fn = &topic.handler;
    let message = &topic.message;
    let owner = &config.owner;
    let provider = &config.provider;

    quote! {
        #[omnia_wasi_otel::instrument]
        async fn #handler_fn(payload: Vec<u8>) -> Result<()> {
             #message::handler(payload)?
                 .provider(&#provider::new())
                 .owner(#owner)
                 .await
                 .map(|_| ())
                 .map_err(Into::into)
        }
    }
}
