use threadpool::ThreadPool;
use promises::Promise;
use bytes::ByteBuf;

enum JobResult<A> {
    Sync {data: A},
    Async {data: Promise<A>}
}

struct EventProcessor {
    pool: ThreadPool
}

impl EventProcessor {
    fn new(cores: usize) -> EventProcessor {
        EventProcessor {
            pool: ThreadPool::new(cores)
        }
    }

    fn execute_sync<F, A, B>(&self, job:F, acceptor: A)
        where F : FnOnce() -> JobResult<B> + Send + 'static ,
        A : FnMut(B),
        B : Send {
        self.pool.execute(move || {
                match job() {
                    JobResult::Sync{..} => println!("Sync result"),
                    JobResult::Async{..} => println!("Async result")
                }
            });
    }
}

#[cfg(test)]
mod test {
    use super::EventProcessor;
    use super::JobResult;
    use bytes::ByteBuf;

    #[test]
    fn test_sync_result() {
        let processor = EventProcessor::new(1);

        processor.execute_sync(move || {println!("Hello World"); JobResult::Sync{data: ByteBuf::new(1)}},
            |result| {println!("Wow")})
    }
}
