use anyhow::anyhow;
use anyhow::Result;
use ethers_providers::{Http, Provider};
use prover::scheduler::{AddServiceResult, Event, TakeTaskResult};
use scheduler_service::scheduler_service_server::SchedulerService;
use scheduler_service::{
    batch_prover_message, scheduler_message, BatchProverMessage, GenBatchProofRequest,
    GetProofResponse, GetStatusResponse, SchedulerMessage,
};
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_stream::{Stream, StreamExt};
use tonic::{Request, Response, Status, Streaming};

pub mod scheduler_service {
    tonic::include_proto!("scheduler.v1");
}

pub struct SchedulerServiceSVC {
    client: Arc<Provider<Http>>,
    scheduler_sender: mpsc::Sender<Event>,
}

impl SchedulerServiceSVC {
    pub fn new(scheduler_sender: mpsc::Sender<Event>) -> Self {
        let url = std::env::var("URL").unwrap_or(String::from("http://localhost:8545"));
        let client = Provider::<Http>::try_from(url).unwrap();
        let client = Arc::new(client);
        SchedulerServiceSVC {
            client,
            scheduler_sender,
        }
    }
}

#[tonic::async_trait]
impl SchedulerService for SchedulerServiceSVC {
    type SchedulerStreamStream =
        Pin<Box<dyn Stream<Item = Result<SchedulerMessage, Status>> + Send + Sync + 'static>>;

    async fn scheduler_stream(
        &self,
        request: Request<Streaming<BatchProverMessage>>,
    ) -> Result<Response<Self::SchedulerStreamStream>, Status> {
        let mut stream = request.into_inner();
        let (tx, rx) = mpsc::channel(10);
        let scheduler_sender = self.scheduler_sender.clone();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    Some(result) = stream.next() => {
                        match result {
                            Ok(batch_prover_msg) => {
                                if let Some(msg) = batch_prover_msg.request {
                                    let resp = match msg {
                                        // update pb, we don't need too much information
                                        batch_prover_message::Request::GetStatusResponse(r) => {
                                            if let Ok(schdeduler_msg) = handle_get_status_response(r, scheduler_sender.clone()).await {
                                                schdeduler_msg
                                            } else {
                                                // close the connection
                                                break;
                                            }
                                        }
                                        // update pb, we don't need GeneBatchProofResponse
                                        // just wait for the result, don't need to get again
                                        batch_prover_message::Request::GenBatchProofResponse(r) => {
                                            if let Ok(scheduler_msg) = handle_gen_batch_proof_response(r.id, scheduler_sender.clone()).await {
                                                scheduler_msg
                                            } else {
                                                // close the connection
                                                break;
                                            }
                                        }
                                        // receive proof, trigger next batch_proof task
                                        batch_prover_message::Request::GetProofResponse(r) => {
                                            if let Ok(scheduler_msg) = handle_get_proof_response(r, scheduler_sender.clone()).await {
                                                scheduler_msg
                                            } else {
                                                // close the connection
                                                break;
                                            }
                                        }
                                        batch_prover_message::Request::CancelRequest(_r) => {
                                            // TODO: cancel the task
                                            todo!()
                                        }
                                    };

                                    if let Err(e) = tx.send(Ok(resp)).await {
                                        log::error!("Failed to send message: {}", e);
                                        break;
                                    }
                                }
                            }
                            Err(e) => {
                                // some error occurred
                                // now, we choose to close the connection
                                // TODO: process according to the status code, eg. retry, close, etc.
                                // send Event::Shutdown to the scheduler, wait for the event result
                                // exit the loop
                                log::error!("Failed to receive message, close: {}", e);
                                // try to send error_message to client?
                                break;
                            }
                        }

                    }
                    else => {
                        // client already closed the connection
                        // send Event::Shutdown to the scheduler,
                        // to notify the scheduler to remove the service
                        // wait for the event result
                        // exit the loop
                        // TODO: put Event::Shutdown ant wait
                        break;
                    }
                }
            }
        });

        Ok(Response::new(Box::pin(
            tokio_stream::wrappers::ReceiverStream::new(rx),
        )))
    }
}

async fn handle_get_status_response(
    r: GetStatusResponse,
    scheduler_sender: mpsc::Sender<Event>,
) -> Result<SchedulerMessage> {
    // send Event::AddService to the scheduler, registry the service to the scheduler
    // wait for the event result from the relay channel
    let (relay_to, mut relay) = mpsc::channel::<AddServiceResult>(1);
    let event = Event::AddService {
        service_id: r.prover_id.clone(),
        relay_to,
    };
    if let Err(e) = scheduler_sender.send(event.clone()).await {
        // can't send event to scheduler, close the connection
        log::error!("Failed to send Event: {:?}, receiver dropped: {}", event, e);
        return Err(anyhow!(
            "Failed to send Event: {:?}, receiver dropped: {}",
            event,
            e
        ));
    }

    if let Some(add_service_result) = relay.recv().await {
        match add_service_result {
            AddServiceResult::Success(_service_id) => {
                if let Ok(scheduler_msg) =
                    handle_gen_batch_proof_response(r.prover_id, scheduler_sender).await
                {
                    Ok(scheduler_msg)
                } else {
                    // close the connection
                    Err(anyhow!("Failed to handle GenBatchProofResponse"))
                }
            }
            AddServiceResult::Fail(service_id) => {
                // close the connection
                Err(anyhow!("Failed to add service: {}", service_id))
            }
        }
    } else {
        // channel closed
        Err(anyhow!(
            "Failed to receive AddServiceResult, channel closed"
        ))
    }
}

async fn handle_gen_batch_proof_response(
    provider_id: String,
    scheduler_sender: mpsc::Sender<Event>,
) -> Result<SchedulerMessage> {
    let (relay_to, mut relay) = mpsc::channel::<TakeTaskResult>(1);
    // then, send Event::TriggerTask to the scheduler
    let event = Event::TakeTask {
        service_id: provider_id.clone(),
        relay_to,
    };

    if let Err(e) = scheduler_sender.send(event.clone()).await {
        // can't send event to scheduler, close the connection
        log::error!("Failed to send Event: {:?}, receiver dropped: {}", event, e);
        return Err(anyhow!(
            "Failed to send Event: {:?}, receiver dropped: {}",
            event,
            e
        ));
    }

    // wait for the event result
    if let Some(take_task_result) = relay.recv().await {
        match take_task_result {
            TakeTaskResult::Success(batch_ctx) => {
                Ok(SchedulerMessage {
                    // TODO: received id
                    id: "uid".into(),
                    response: Some(
                        // TODO: send BatchContext
                        scheduler_message::Response::GenBatchProofRequest(GenBatchProofRequest {
                            input: None,
                            execute_task_id: batch_ctx.task_id,
                            chunk_id: batch_ctx.chunk_id,
                        }),
                    ),
                })
            }
            TakeTaskResult::Fail(service_id) => {
                Err(anyhow!("Failed to take task for service: {}", service_id))
            }
        }
    } else {
        // channel closed
        Err(anyhow!("Failed to receive TakeTaskResult, channel closed"))
    }
}

async fn handle_get_proof_response(
    r: GetProofResponse,
    scheduler_sender: mpsc::Sender<Event>,
) -> Result<SchedulerMessage> {
    let event = Event::TaskResult {
        service_id: r.id.clone(),
        recursive_proof: r.recursive_proof.clone(),
    };
    if let Err(e) = scheduler_sender.send(event.clone()).await {
        // can't send event to scheduler, close the connection
        log::error!("Failed to send Event: {:?}, receiver dropped: {}", event, e);
        return Err(anyhow!(
            "Failed to send Event: {:?}, receiver dropped: {}",
            event,
            e
        ));
    }

    if let Ok(scheduler_msg) = handle_gen_batch_proof_response(r.id, scheduler_sender).await {
        Ok(scheduler_msg)
    } else {
        // close the connection
        Err(anyhow!("Failed to handle GenBatchProofResponse"))
    }
}