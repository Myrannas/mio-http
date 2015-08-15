use std::error::Error;
use std::boxed::FnBox;
use std::sync::mpsc::{Receiver, Sender, RecvError, channel};

#[derive(Clone)]
pub enum ExecutionContext {
    ImmediateContext
}

impl ExecutionContext {
    fn execute<F>(&self, f: F) where F : FnOnce() {
        match &self {
            &ImmediateContext => {
                f();
            }
        }
    }
}

struct PromiseFactory {
    execution_context: ExecutionContext
}

pub fn incomplete<A>() -> Promise<A>{
    PromiseFactory{execution_context: ExecutionContext::ImmediateContext}.incomplete()
}

pub fn completed<A>(a: Result<A, Box<Error>>) -> Promise<A> {
    PromiseFactory{execution_context: ExecutionContext::ImmediateContext}.completed(a)
}

impl PromiseFactory {
    pub fn incomplete<A>(&self) -> Promise<A> {
        Promise{
            data: None,
            success_callbacks: Vec::with_capacity(1),
            failure_callbacks: Vec::with_capacity(1),
            execution_context: self.execution_context.clone()
        }
    }

    pub fn completed<A>(&self, a: Result<A, Box<Error>>) -> Promise<A> {
        Promise{
            data: Some(a),
            success_callbacks: Vec::with_capacity(1),
            failure_callbacks: Vec::with_capacity(1),
            execution_context: self.execution_context.clone()
        }
    }
}

struct Promise<A: Sized> {
    data: Option<Result<A, Box<Error>>>,
    success_callbacks: Vec<Box<FnBox(&A)>>,
    failure_callbacks : Vec<Box<FnBox(&Error)>>,
    execution_context: ExecutionContext
}

impl <A:Sized> Promise<A> {
    pub fn success<F>(&mut self, on_success: F) where F: FnOnce(&A)->() + Send + 'static {
        match self.data {
            Some(Ok(ref d)) => {
                on_success(d)
            },
            Some(Err(_))=> (),
            None => {
                self.success_callbacks.push(Box::new(on_success))
            }
        }
    }

    pub fn failure<F>(&mut self, on_failure: F) where F: FnOnce(&Error) + Send + 'static {
        match self.data {
            Some(Err(ref e)) => {
                let error = & **e;
                on_failure(error)
            },
            Some(Ok(_)) => (),
            None => {
                self.failure_callbacks.push(Box::new(on_failure))
            }
        }
    }

    pub fn complete(&mut self, a: A) {
        let callbacks = self.success_callbacks.len();
        let range = 0 .. callbacks;
        for f in self.success_callbacks.drain(range) {
            f.call_once((&a,))
        }

        self.data = Some(Ok(a));
    }

    pub fn fail(&mut self, err: Box<Error>) {

        let callbacks = self.failure_callbacks.len();
        let range = 0 .. callbacks;

        print!("Invoking error callback {:?}", callbacks);

        for f in self.failure_callbacks.drain(range) {
            let error_pointer = & * err;
            f.call_once((error_pointer,))
        }

        self.data = Some(Err(err));
    }

    pub fn map<F,B>(&mut self, map: F) -> Promise<B> where
        F : FnOnce(&A) -> B + Send + 'static,
        B : Send {

        let mut p = incomplete();
        self.success(move |a| {
                p.complete(map(a))
            });
        p
    }
}

#[cfg(test)]
mod test {
    use super::Promise;
    use std::error::Error;
    use std::sync::mpsc::{Receiver, Sender, RecvError, channel};
    use std::fmt::{Display, Formatter};
    use core::fmt::Error as FmtError;

    #[derive(Debug)]
    struct TestError;
    impl Error for TestError { fn description(&self) -> &'static str {"Error"} }
    impl Display for TestError {
        fn fmt(&self, fmt: &mut Formatter) -> Result<(), FmtError> { Ok(()) }
    }

    #[test]
    fn test_promise_complete() {
        let mut promise = Promise::incomplete();
        let (tx, rx): (Sender<i32>, Receiver<i32>) = channel();

        promise.success(move |d| {
                tx.send((*d) + 1);
                ()
            });

        promise.complete(1);

        assert_eq!(rx.recv().unwrap(), 2);
    }

    #[test]
    fn test_promise_immediately_completed() {
        let mut promise = Promise::completed(Ok(1));
        let (tx, rx): (Sender<i32>, Receiver<i32>) = channel();

        promise.success(move |d| {
                tx.send((*d) + 1);
                ()
            });

        assert_eq!(rx.recv().unwrap(), 2);
    }

    #[test]
    fn test_promise_failed_immediate() {
        let mut promise : Promise<u32> = Promise::completed(Err(Box::new(TestError)));
        let (tx, rx): (Sender<String>, Receiver<String>) = channel();

        promise.failure(move |err: &Error| {
                tx.send(String::from(err.description()));
                ()
            });

        assert_eq!(rx.recv().unwrap(), "Error");
    }

    #[test]
    fn test_promise_fail() {
        let mut promise : Promise<u32> = Promise::incomplete();
        let (tx, rx): (Sender<String>, Receiver<String>) = channel();

        promise.failure(move |err: &Error| {
                println!("Failing");
                tx.send(String::from(err.description()));
                ()
            });

        promise.fail(Box::new(TestError));

        assert_eq!(rx.recv().unwrap(), "Error");
    }

    #[test]
    fn test_promise_map() {
        let mut promise : Promise<u32> = Promise::incomplete();
        let (tx, rx): (Sender<String>, Receiver<String>) = channel();

        promise.failure(move |err: &Error| {
                println!("Failing");
                tx.send(String::from(err.description()));
                ()
            });

        promise.fail(Box::new(TestError));

        assert_eq!(rx.recv().unwrap(), "Error");
    }
}
