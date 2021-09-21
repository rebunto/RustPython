pub(crate) use decl::make_module;

#[pymodule(name = "itertools")]
mod decl {
    use crate::common::{
        lock::{PyMutex, PyRwLock, PyRwLockWriteGuard},
        rc::PyRc,
    };
    use crate::{
        builtins::{int, PyInt, PyIntRef, PyTupleRef, PyTypeRef},
        function::{Args, FuncArgs, OptionalArg, OptionalOption},
        iterator::{call_next, get_iter, get_next_object},
        slots::{PyIter, SlotConstructor},
        ArgCallable, IdProtocol, IntoPyObject, PyObjectRef, PyRef, PyResult, PyValue, PyWeakRef,
        StaticType, TypeProtocol, VirtualMachine,
    };
    use crossbeam_utils::atomic::AtomicCell;
    use num_bigint::BigInt;
    use num_traits::{One, Signed, ToPrimitive, Zero};
    use std::fmt;

    #[pyattr]
    #[pyclass(name = "chain")]
    #[derive(Debug, PyValue)]
    struct PyItertoolsChain {
        iterables: Vec<PyObjectRef>,
        cur_idx: AtomicCell<usize>,
        cached_iter: PyRwLock<Option<PyObjectRef>>,
    }

    #[pyimpl(with(PyIter))]
    impl PyItertoolsChain {
        #[pyslot]
        fn tp_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            PyItertoolsChain {
                iterables: args.args,
                cur_idx: AtomicCell::new(0),
                cached_iter: PyRwLock::new(None),
            }
            .into_pyresult_with_type(vm, cls)
        }

        #[pyclassmethod]
        fn from_iterable(
            cls: PyTypeRef,
            iterable: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            PyItertoolsChain {
                iterables: vm.extract_elements(&iterable)?,
                cur_idx: AtomicCell::new(0),
                cached_iter: PyRwLock::new(None),
            }
            .into_ref_with_type(vm, cls)
        }
    }
    impl PyIter for PyItertoolsChain {
        fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            loop {
                let pos = zelf.cur_idx.load();
                if pos >= zelf.iterables.len() {
                    break;
                }
                let cur_iter = if zelf.cached_iter.read().is_none() {
                    // We need to call "get_iter" outside of the lock.
                    let iter = get_iter(vm, zelf.iterables[pos].clone())?;
                    *zelf.cached_iter.write() = Some(iter.clone());
                    iter
                } else if let Some(cached_iter) = (*zelf.cached_iter.read()).clone() {
                    cached_iter
                } else {
                    // Someone changed cached iter to None since we checked.
                    continue;
                };

                // We need to call "call_next" outside of the lock.
                match call_next(vm, &cur_iter) {
                    Ok(ok) => return Ok(ok),
                    Err(err) => {
                        if err.isinstance(&vm.ctx.exceptions.stop_iteration) {
                            zelf.cur_idx.fetch_add(1);
                            *zelf.cached_iter.write() = None;
                        } else {
                            return Err(err);
                        }
                    }
                }
            }

            Err(vm.new_stop_iteration())
        }
    }

    #[pyattr]
    #[pyclass(name = "compress")]
    #[derive(Debug, PyValue)]
    struct PyItertoolsCompress {
        data: PyObjectRef,
        selector: PyObjectRef,
    }

    #[derive(FromArgs)]
    struct CompressNewArgs {
        #[pyarg(positional)]
        data: PyObjectRef,
        #[pyarg(positional)]
        selector: PyObjectRef,
    }

    impl SlotConstructor for PyItertoolsCompress {
        type Args = CompressNewArgs;

        fn py_new(
            cls: PyTypeRef,
            Self::Args { data, selector }: Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult {
            let data_iter = get_iter(vm, data)?;
            let selector_iter = get_iter(vm, selector)?;

            PyItertoolsCompress {
                data: data_iter,
                selector: selector_iter,
            }
            .into_pyresult_with_type(vm, cls)
        }
    }

    #[pyimpl(with(PyIter, SlotConstructor))]
    impl PyItertoolsCompress {}

    impl PyIter for PyItertoolsCompress {
        fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            loop {
                let sel_obj = call_next(vm, &zelf.selector)?;
                let verdict = sel_obj.clone().try_to_bool(vm)?;
                let data_obj = call_next(vm, &zelf.data)?;

                if verdict {
                    return Ok(data_obj);
                }
            }
        }
    }

    #[pyattr]
    #[pyclass(name = "count")]
    #[derive(Debug, PyValue)]
    struct PyItertoolsCount {
        cur: PyRwLock<BigInt>,
        step: BigInt,
    }

    #[derive(FromArgs)]
    struct CountNewArgs {
        #[pyarg(positional, optional)]
        start: OptionalArg<PyIntRef>,

        #[pyarg(positional, optional)]
        step: OptionalArg<PyIntRef>,
    }

    impl SlotConstructor for PyItertoolsCount {
        type Args = CountNewArgs;

        fn py_new(
            cls: PyTypeRef,
            Self::Args { start, step }: Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult {
            let start = match start.into_option() {
                Some(int) => int.as_bigint().clone(),
                None => BigInt::zero(),
            };
            let step = match step.into_option() {
                Some(int) => int.as_bigint().clone(),
                None => BigInt::one(),
            };

            PyItertoolsCount {
                cur: PyRwLock::new(start),
                step,
            }
            .into_pyresult_with_type(vm, cls)
        }
    }

    #[pyimpl(with(PyIter, SlotConstructor))]
    impl PyItertoolsCount {}
    impl PyIter for PyItertoolsCount {
        fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            let mut cur = zelf.cur.write();
            let result = cur.clone();
            *cur += &zelf.step;
            Ok(result.into_pyobject(vm))
        }
    }

    #[pyattr]
    #[pyclass(name = "cycle")]
    #[derive(Debug, PyValue)]
    struct PyItertoolsCycle {
        iter: PyObjectRef,
        saved: PyRwLock<Vec<PyObjectRef>>,
        index: AtomicCell<usize>,
    }

    impl SlotConstructor for PyItertoolsCycle {
        type Args = PyObjectRef;

        fn py_new(cls: PyTypeRef, iterable: Self::Args, vm: &VirtualMachine) -> PyResult {
            let iter = get_iter(vm, iterable)?;

            PyItertoolsCycle {
                iter,
                saved: PyRwLock::new(Vec::new()),
                index: AtomicCell::new(0),
            }
            .into_pyresult_with_type(vm, cls)
        }
    }

    #[pyimpl(with(PyIter, SlotConstructor))]
    impl PyItertoolsCycle {}
    impl PyIter for PyItertoolsCycle {
        fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            let item = if let Some(item) = get_next_object(vm, &zelf.iter)? {
                zelf.saved.write().push(item.clone());
                item
            } else {
                let saved = zelf.saved.read();
                if saved.len() == 0 {
                    return Err(vm.new_stop_iteration());
                }

                let last_index = zelf.index.fetch_add(1);

                if last_index >= saved.len() - 1 {
                    zelf.index.store(0);
                }

                saved[last_index].clone()
            };

            Ok(item)
        }
    }

    #[pyattr]
    #[pyclass(name = "repeat")]
    #[derive(Debug, PyValue)]
    struct PyItertoolsRepeat {
        object: PyObjectRef,
        times: Option<PyRwLock<usize>>,
    }

    #[derive(FromArgs)]
    struct PyRepeatNewArgs {
        #[pyarg(any)]
        object: PyObjectRef,
        #[pyarg(any, optional)]
        times: OptionalArg<PyIntRef>,
    }

    impl SlotConstructor for PyItertoolsRepeat {
        type Args = PyRepeatNewArgs;

        fn py_new(
            cls: PyTypeRef,
            Self::Args { object, times }: Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult {
            let times = match times.into_option() {
                Some(int) => {
                    let val = int.as_bigint();
                    if *val > BigInt::from(isize::MAX) {
                        return Err(vm.new_overflow_error("Cannot fit in isize.".to_owned()));
                    }
                    // times always >= 0.
                    Some(PyRwLock::new(val.to_usize().unwrap_or(0)))
                }
                None => None,
            };
            PyItertoolsRepeat { object, times }.into_pyresult_with_type(vm, cls)
        }
    }

    #[pyimpl(with(PyIter, SlotConstructor), flags(BASETYPE))]
    impl PyItertoolsRepeat {
        #[pymethod(magic)]
        fn length_hint(&self, vm: &VirtualMachine) -> PyResult {
            match self.times {
                Some(ref times) => Ok(vm.ctx.new_int(*times.read())),
                // Return TypeError, length_hint picks this up and returns the default.
                None => Err(vm.new_type_error("length of unsized object.".to_owned())),
            }
        }

        #[pymethod(magic)]
        fn reduce(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            let cls = zelf.clone_class().into_pyobject(vm);
            Ok(match zelf.times {
                Some(ref times) => vm.ctx.new_tuple(vec![
                    cls,
                    vm.ctx.new_tuple(vec![
                        zelf.object.clone(),
                        vm.ctx.new_int(*times.read()).into_pyobject(vm),
                    ]),
                ]),
                None => vm
                    .ctx
                    .new_tuple(vec![cls, vm.ctx.new_tuple(vec![zelf.object.clone()])]),
            })
        }

        #[pymethod(magic)]
        fn repr(&self, vm: &VirtualMachine) -> PyResult<String> {
            let mut fmt = format!("{}", vm.to_repr(&self.object)?);
            if let Some(ref times) = self.times {
                fmt.push_str(&format!(", {}", times.read()));
            }
            Ok(format!("repeat({})", fmt))
        }
    }

    impl PyIter for PyItertoolsRepeat {
        fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            if let Some(ref times) = zelf.times {
                let mut times = times.write();
                if *times == 0 {
                    return Err(vm.new_stop_iteration());
                }
                *times -= 1;
            }
            Ok(zelf.object.clone())
        }
    }

    #[pyattr]
    #[pyclass(name = "starmap")]
    #[derive(Debug, PyValue)]
    struct PyItertoolsStarmap {
        function: PyObjectRef,
        iter: PyObjectRef,
    }

    #[derive(FromArgs)]
    struct StarmapNewArgs {
        #[pyarg(positional)]
        function: PyObjectRef,
        #[pyarg(positional)]
        iterable: PyObjectRef,
    }

    impl SlotConstructor for PyItertoolsStarmap {
        type Args = StarmapNewArgs;

        fn py_new(
            cls: PyTypeRef,
            Self::Args { function, iterable }: Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult {
            let iter = get_iter(vm, iterable)?;

            PyItertoolsStarmap { function, iter }.into_pyresult_with_type(vm, cls)
        }
    }

    #[pyimpl(with(PyIter, SlotConstructor))]
    impl PyItertoolsStarmap {}
    impl PyIter for PyItertoolsStarmap {
        fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            let obj = call_next(vm, &zelf.iter)?;
            let function = &zelf.function;

            vm.invoke(function, vm.extract_elements(&obj)?)
        }
    }

    #[pyattr]
    #[pyclass(name = "takewhile")]
    #[derive(Debug, PyValue)]
    struct PyItertoolsTakewhile {
        predicate: PyObjectRef,
        iterable: PyObjectRef,
        stop_flag: AtomicCell<bool>,
    }

    #[derive(FromArgs)]
    struct TakewhileNewArgs {
        #[pyarg(positional)]
        predicate: PyObjectRef,
        #[pyarg(positional)]
        iterable: PyObjectRef,
    }

    impl SlotConstructor for PyItertoolsTakewhile {
        type Args = TakewhileNewArgs;

        fn py_new(
            cls: PyTypeRef,
            Self::Args {
                predicate,
                iterable,
            }: Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult {
            let iter = get_iter(vm, iterable)?;

            PyItertoolsTakewhile {
                predicate,
                iterable: iter,
                stop_flag: AtomicCell::new(false),
            }
            .into_pyresult_with_type(vm, cls)
        }
    }

    #[pyimpl(with(PyIter, SlotConstructor))]
    impl PyItertoolsTakewhile {}
    impl PyIter for PyItertoolsTakewhile {
        fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            if zelf.stop_flag.load() {
                return Err(vm.new_stop_iteration());
            }

            // might be StopIteration or anything else, which is propagated upwards
            let obj = call_next(vm, &zelf.iterable)?;
            let predicate = &zelf.predicate;

            let verdict = vm.invoke(predicate, (obj.clone(),))?;
            let verdict = verdict.try_to_bool(vm)?;
            if verdict {
                Ok(obj)
            } else {
                zelf.stop_flag.store(true);
                Err(vm.new_stop_iteration())
            }
        }
    }

    #[pyattr]
    #[pyclass(name = "dropwhile")]
    #[derive(Debug, PyValue)]
    struct PyItertoolsDropwhile {
        predicate: ArgCallable,
        iterable: PyObjectRef,
        start_flag: AtomicCell<bool>,
    }

    #[derive(FromArgs)]
    struct DropwhileNewArgs {
        #[pyarg(positional)]
        predicate: ArgCallable,
        #[pyarg(positional)]
        iterable: PyObjectRef,
    }

    impl SlotConstructor for PyItertoolsDropwhile {
        type Args = DropwhileNewArgs;

        fn py_new(
            cls: PyTypeRef,
            Self::Args {
                predicate,
                iterable,
            }: Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult {
            let iter = get_iter(vm, iterable)?;

            PyItertoolsDropwhile {
                predicate,
                iterable: iter,
                start_flag: AtomicCell::new(false),
            }
            .into_pyresult_with_type(vm, cls)
        }
    }

    #[pyimpl(with(PyIter, SlotConstructor))]
    impl PyItertoolsDropwhile {}
    impl PyIter for PyItertoolsDropwhile {
        fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            let predicate = &zelf.predicate;
            let iterable = &zelf.iterable;

            if !zelf.start_flag.load() {
                loop {
                    let obj = call_next(vm, iterable)?;
                    let pred = predicate.clone();
                    let pred_value = vm.invoke(&pred.into_object(), (obj.clone(),))?;
                    if !pred_value.try_to_bool(vm)? {
                        zelf.start_flag.store(true);
                        return Ok(obj);
                    }
                }
            }
            call_next(vm, iterable)
        }
    }

    struct GroupByState {
        current_value: Option<PyObjectRef>,
        current_key: Option<PyObjectRef>,
        next_group: bool,
        grouper: Option<PyWeakRef<PyItertoolsGrouper>>,
    }

    impl fmt::Debug for GroupByState {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("GroupByState")
                .field("current_value", &self.current_value)
                .field("current_key", &self.current_key)
                .field("next_group", &self.next_group)
                .finish()
        }
    }

    impl GroupByState {
        fn is_current(&self, grouper: &PyItertoolsGrouperRef) -> bool {
            self.grouper
                .as_ref()
                .and_then(|g| g.upgrade())
                .map_or(false, |ref current_grouper| grouper.is(current_grouper))
        }
    }

    #[pyattr]
    #[pyclass(name = "groupby")]
    #[derive(PyValue)]
    struct PyItertoolsGroupBy {
        iterable: PyObjectRef,
        key_func: Option<PyObjectRef>,
        state: PyMutex<GroupByState>,
    }

    impl fmt::Debug for PyItertoolsGroupBy {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("PyItertoolsGroupBy")
                .field("iterable", &self.iterable)
                .field("key_func", &self.key_func)
                .field("state", &self.state.lock())
                .finish()
        }
    }

    #[derive(FromArgs)]
    struct GroupByArgs {
        iterable: PyObjectRef,
        #[pyarg(any, optional)]
        key: OptionalOption<PyObjectRef>,
    }

    impl SlotConstructor for PyItertoolsGroupBy {
        type Args = GroupByArgs;

        fn py_new(cls: PyTypeRef, args: Self::Args, vm: &VirtualMachine) -> PyResult {
            let iter = get_iter(vm, args.iterable)?;

            PyItertoolsGroupBy {
                iterable: iter,
                key_func: args.key.flatten(),
                state: PyMutex::new(GroupByState {
                    current_key: None,
                    current_value: None,
                    next_group: false,
                    grouper: None,
                }),
            }
            .into_pyresult_with_type(vm, cls)
        }
    }

    #[pyimpl(with(PyIter, SlotConstructor))]
    impl PyItertoolsGroupBy {
        pub(super) fn advance(&self, vm: &VirtualMachine) -> PyResult<(PyObjectRef, PyObjectRef)> {
            let new_value = call_next(vm, &self.iterable)?;
            let new_key = if let Some(ref kf) = self.key_func {
                vm.invoke(kf, vec![new_value.clone()])?
            } else {
                new_value.clone()
            };
            Ok((new_value, new_key))
        }
    }
    impl PyIter for PyItertoolsGroupBy {
        fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            let mut state = zelf.state.lock();
            state.grouper = None;

            if !state.next_group {
                // FIXME: unnecessary clone. current_key always exist until assigning new
                let current_key = state.current_key.clone();
                drop(state);

                let (value, key) = if let Some(old_key) = current_key {
                    loop {
                        let (value, new_key) = zelf.advance(vm)?;
                        if !vm.bool_eq(&new_key, &old_key)? {
                            break (value, new_key);
                        }
                    }
                } else {
                    zelf.advance(vm)?
                };

                state = zelf.state.lock();
                state.current_value = Some(value);
                state.current_key = Some(key);
            }

            state.next_group = false;

            let grouper = PyItertoolsGrouper {
                groupby: zelf.clone(),
            }
            .into_ref(vm);

            state.grouper = Some(PyRef::downgrade(&grouper));
            Ok((state.current_key.as_ref().unwrap().clone(), grouper).into_pyobject(vm))
        }
    }

    #[pyattr]
    #[pyclass(name = "_grouper")]
    #[derive(Debug, PyValue)]
    struct PyItertoolsGrouper {
        groupby: PyRef<PyItertoolsGroupBy>,
    }

    type PyItertoolsGrouperRef = PyRef<PyItertoolsGrouper>;

    #[pyimpl(with(PyIter))]
    impl PyItertoolsGrouper {}
    impl PyIter for PyItertoolsGrouper {
        fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            let old_key = {
                let mut state = zelf.groupby.state.lock();

                if !state.is_current(zelf) {
                    return Err(vm.new_stop_iteration());
                }

                // check to see if the value has already been retrieved from the iterator
                if let Some(val) = state.current_value.take() {
                    return Ok(val);
                }

                state.current_key.as_ref().unwrap().clone()
            };
            let (value, key) = zelf.groupby.advance(vm)?;
            if vm.bool_eq(&key, &old_key)? {
                Ok(value)
            } else {
                let mut state = zelf.groupby.state.lock();
                state.current_value = Some(value);
                state.current_key = Some(key);
                state.next_group = true;
                state.grouper = None;
                Err(vm.new_stop_iteration())
            }
        }
    }

    #[pyattr]
    #[pyclass(name = "islice")]
    #[derive(Debug, PyValue)]
    struct PyItertoolsIslice {
        iterable: PyObjectRef,
        cur: AtomicCell<usize>,
        next: AtomicCell<usize>,
        stop: Option<usize>,
        step: usize,
    }

    // Restrict obj to ints with value 0 <= val <= sys.maxsize (isize::MAX).
    // On failure (out of range, non-int object) a ValueError is raised.
    fn pyobject_to_opt_usize(
        obj: PyObjectRef,
        name: &'static str,
        vm: &VirtualMachine,
    ) -> PyResult<usize> {
        let is_int = obj.isinstance(&vm.ctx.types.int_type);
        if is_int {
            let value = int::get_value(&obj).to_usize();
            if let Some(value) = value {
                // Only succeeds for values for which 0 <= value <= isize::MAX
                if value <= isize::MAX as usize {
                    return Ok(value);
                }
            }
        }
        // We don't have an int or value was < 0 or > maxsize (isize::MAX)
        return Err(vm.new_value_error(format!(
            "{} argument for islice() must be None or an integer: 0 <= x <= sys.maxsize.",
            name
        )));
    }

    #[pyimpl(with(PyIter))]
    impl PyItertoolsIslice {
        #[pyslot]
        fn tp_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            let (iter, start, stop, step) = match args.args.len() {
                0 | 1 => {
                    return Err(vm.new_type_error(format!(
                        "islice expected at least 2 arguments, got {}",
                        args.args.len()
                    )));
                }
                2 => {
                    let (iter, stop): (PyObjectRef, PyObjectRef) = args.bind(vm)?;
                    (iter, 0usize, stop, 1usize)
                }
                _ => {
                    let (iter, start, stop, step) = if args.args.len() == 3 {
                        let (iter, start, stop): (PyObjectRef, PyObjectRef, PyObjectRef) =
                            args.bind(vm)?;
                        (iter, start, stop, 1usize)
                    } else {
                        let (iter, start, stop, step): (
                            PyObjectRef,
                            PyObjectRef,
                            PyObjectRef,
                            PyObjectRef,
                        ) = args.bind(vm)?;

                        let step = if !vm.is_none(&step) {
                            pyobject_to_opt_usize(step, "Step", vm)?
                        } else {
                            1usize
                        };
                        (iter, start, stop, step)
                    };
                    let start = if !vm.is_none(&start) {
                        pyobject_to_opt_usize(start, "Start", vm)?
                    } else {
                        0usize
                    };

                    (iter, start, stop, step)
                }
            };

            let stop = if !vm.is_none(&stop) {
                Some(pyobject_to_opt_usize(stop, "Stop", vm)?)
            } else {
                None
            };

            let iter = get_iter(vm, iter)?;

            PyItertoolsIslice {
                iterable: iter,
                cur: AtomicCell::new(0),
                next: AtomicCell::new(start),
                stop,
                step,
            }
            .into_pyresult_with_type(vm, cls)
        }
    }

    impl PyIter for PyItertoolsIslice {
        fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            while zelf.cur.load() < zelf.next.load() {
                call_next(vm, &zelf.iterable)?;
                zelf.cur.fetch_add(1);
            }

            if let Some(stop) = zelf.stop {
                if zelf.cur.load() >= stop {
                    return Err(vm.new_stop_iteration());
                }
            }

            let obj = call_next(vm, &zelf.iterable)?;
            zelf.cur.fetch_add(1);

            // TODO is this overflow check required? attempts to copy CPython.
            let (next, ovf) = zelf.next.load().overflowing_add(zelf.step);
            zelf.next.store(if ovf { zelf.stop.unwrap() } else { next });

            Ok(obj)
        }
    }

    #[pyattr]
    #[pyclass(name = "filterfalse")]
    #[derive(Debug, PyValue)]
    struct PyItertoolsFilterFalse {
        predicate: PyObjectRef,
        iterable: PyObjectRef,
    }

    #[derive(FromArgs)]
    struct FilterFalseNewArgs {
        #[pyarg(positional)]
        predicate: PyObjectRef,
        #[pyarg(positional)]
        iterable: PyObjectRef,
    }

    impl SlotConstructor for PyItertoolsFilterFalse {
        type Args = FilterFalseNewArgs;

        fn py_new(
            cls: PyTypeRef,
            Self::Args {
                predicate,
                iterable,
            }: Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult {
            let iter = get_iter(vm, iterable)?;

            PyItertoolsFilterFalse {
                predicate,
                iterable: iter,
            }
            .into_pyresult_with_type(vm, cls)
        }
    }

    #[pyimpl(with(PyIter, SlotConstructor))]
    impl PyItertoolsFilterFalse {}
    impl PyIter for PyItertoolsFilterFalse {
        fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            let predicate = &zelf.predicate;
            let iterable = &zelf.iterable;

            loop {
                let obj = call_next(vm, iterable)?;
                let pred_value = if vm.is_none(predicate) {
                    obj.clone()
                } else {
                    vm.invoke(predicate, vec![obj.clone()])?
                };

                if !pred_value.try_to_bool(vm)? {
                    return Ok(obj);
                }
            }
        }
    }

    #[pyattr]
    #[pyclass(name = "accumulate")]
    #[derive(Debug, PyValue)]
    struct PyItertoolsAccumulate {
        iterable: PyObjectRef,
        binop: Option<PyObjectRef>,
        initial: Option<PyObjectRef>,
        acc_value: PyRwLock<Option<PyObjectRef>>,
    }

    #[derive(FromArgs)]
    struct AccumulateArgs {
        iterable: PyObjectRef,
        #[pyarg(any, optional)]
        func: OptionalOption<PyObjectRef>,
        #[pyarg(named, optional)]
        initial: OptionalOption<PyObjectRef>,
    }

    impl SlotConstructor for PyItertoolsAccumulate {
        type Args = AccumulateArgs;

        fn py_new(cls: PyTypeRef, args: AccumulateArgs, vm: &VirtualMachine) -> PyResult {
            let iter = get_iter(vm, args.iterable)?;

            PyItertoolsAccumulate {
                iterable: iter,
                binop: args.func.flatten(),
                initial: args.initial.flatten(),
                acc_value: PyRwLock::new(None),
            }
            .into_pyresult_with_type(vm, cls)
        }
    }

    #[pyimpl(with(PyIter, SlotConstructor))]
    impl PyItertoolsAccumulate {}

    impl PyIter for PyItertoolsAccumulate {
        fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            let iterable = &zelf.iterable;

            let acc_value = zelf.acc_value.read().clone();

            let next_acc_value = match acc_value {
                None => match &zelf.initial {
                    None => call_next(vm, iterable)?,
                    Some(obj) => obj.clone(),
                },
                Some(value) => {
                    let obj = call_next(vm, iterable)?;
                    match &zelf.binop {
                        None => vm._add(&value, &obj)?,
                        Some(op) => vm.invoke(op, vec![value, obj])?,
                    }
                }
            };
            *zelf.acc_value.write() = Some(next_acc_value.clone());

            Ok(next_acc_value)
        }
    }

    #[derive(Debug)]
    struct PyItertoolsTeeData {
        iterable: PyObjectRef,
        values: PyRwLock<Vec<PyObjectRef>>,
    }

    impl PyItertoolsTeeData {
        fn new(iterable: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyRc<PyItertoolsTeeData>> {
            Ok(PyRc::new(PyItertoolsTeeData {
                iterable: get_iter(vm, iterable)?,
                values: PyRwLock::new(vec![]),
            }))
        }

        fn get_item(&self, vm: &VirtualMachine, index: usize) -> PyResult {
            if self.values.read().len() == index {
                let result = call_next(vm, &self.iterable)?;
                self.values.write().push(result);
            }
            Ok(self.values.read()[index].clone())
        }
    }

    #[pyattr]
    #[pyclass(name = "tee")]
    #[derive(Debug, PyValue)]
    struct PyItertoolsTee {
        tee_data: PyRc<PyItertoolsTeeData>,
        index: AtomicCell<usize>,
    }

    #[derive(FromArgs)]
    struct TeeNewArgs {
        #[pyarg(positional)]
        iterable: PyObjectRef,
        #[pyarg(positional, optional)]
        n: OptionalArg<usize>,
    }

    impl SlotConstructor for PyItertoolsTee {
        type Args = TeeNewArgs;

        // TODO: make tee() a function, rename this class to itertools._tee and make
        // teedata a python class
        #[allow(clippy::new_ret_no_self)]
        fn py_new(
            _cls: PyTypeRef,
            Self::Args { iterable, n }: Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult {
            let n = n.unwrap_or(2);

            let copyable = if iterable.class().has_attr("__copy__") {
                vm.call_method(&iterable, "__copy__", ())?
            } else {
                PyItertoolsTee::from_iter(iterable, vm)?
            };

            let mut tee_vec: Vec<PyObjectRef> = Vec::with_capacity(n);
            for _ in 0..n {
                tee_vec.push(vm.call_method(&copyable, "__copy__", ())?);
            }

            Ok(PyTupleRef::with_elements(tee_vec, &vm.ctx).into_object())
        }
    }

    #[pyimpl(with(PyIter, SlotConstructor))]
    impl PyItertoolsTee {
        fn from_iter(iterable: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            let class = PyItertoolsTee::class(vm);
            let it = get_iter(vm, iterable)?;
            if it.class().is(PyItertoolsTee::class(vm)) {
                return vm.call_method(&it, "__copy__", ());
            }
            Ok(PyItertoolsTee {
                tee_data: PyItertoolsTeeData::new(it, vm)?,
                index: AtomicCell::new(0),
            }
            .into_ref_with_type(vm, class.clone())?
            .into_object())
        }

        #[pymethod(magic)]
        fn copy(&self, vm: &VirtualMachine) -> PyResult {
            Ok(PyItertoolsTee {
                tee_data: PyRc::clone(&self.tee_data),
                index: AtomicCell::new(self.index.load()),
            }
            .into_ref_with_type(vm, Self::class(vm).clone())?
            .into_object())
        }
    }
    impl PyIter for PyItertoolsTee {
        fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            let value = zelf.tee_data.get_item(vm, zelf.index.load())?;
            zelf.index.fetch_add(1);
            Ok(value)
        }
    }

    #[pyattr]
    #[pyclass(name = "product")]
    #[derive(Debug, PyValue)]
    struct PyItertoolsProduct {
        pools: Vec<Vec<PyObjectRef>>,
        idxs: PyRwLock<Vec<usize>>,
        cur: AtomicCell<usize>,
        stop: AtomicCell<bool>,
    }

    #[derive(FromArgs)]
    struct ProductArgs {
        #[pyarg(named, optional)]
        repeat: OptionalArg<usize>,
    }

    impl SlotConstructor for PyItertoolsProduct {
        type Args = (Args<PyObjectRef>, ProductArgs);

        fn py_new(cls: PyTypeRef, (iterables, args): Self::Args, vm: &VirtualMachine) -> PyResult {
            let repeat = args.repeat.unwrap_or(1);
            let mut pools = Vec::new();
            for arg in iterables.iter() {
                pools.push(vm.extract_elements(arg)?);
            }
            let pools = std::iter::repeat(pools)
                .take(repeat)
                .flatten()
                .collect::<Vec<Vec<PyObjectRef>>>();

            let l = pools.len();

            PyItertoolsProduct {
                pools,
                idxs: PyRwLock::new(vec![0; l]),
                cur: AtomicCell::new(l.wrapping_sub(1)),
                stop: AtomicCell::new(false),
            }
            .into_pyresult_with_type(vm, cls)
        }
    }

    #[pyimpl(with(PyIter, SlotConstructor))]
    impl PyItertoolsProduct {
        fn update_idxs(&self, mut idxs: PyRwLockWriteGuard<'_, Vec<usize>>) {
            if idxs.len() == 0 {
                self.stop.store(true);
                return;
            }

            let cur = self.cur.load();
            let lst_idx = &self.pools[cur].len() - 1;

            if idxs[cur] == lst_idx {
                if cur == 0 {
                    self.stop.store(true);
                    return;
                }
                idxs[cur] = 0;
                self.cur.fetch_sub(1);
                self.update_idxs(idxs);
            } else {
                idxs[cur] += 1;
                self.cur.store(idxs.len() - 1);
            }
        }
    }
    impl PyIter for PyItertoolsProduct {
        fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            // stop signal
            if zelf.stop.load() {
                return Err(vm.new_stop_iteration());
            }

            let pools = &zelf.pools;

            for p in pools {
                if p.is_empty() {
                    return Err(vm.new_stop_iteration());
                }
            }

            let idxs = zelf.idxs.write();
            let res = vm.ctx.new_tuple(
                pools
                    .iter()
                    .zip(idxs.iter())
                    .map(|(pool, idx)| pool[*idx].clone())
                    .collect(),
            );

            zelf.update_idxs(idxs);

            Ok(res)
        }
    }

    #[pyattr]
    #[pyclass(name = "combinations")]
    #[derive(Debug, PyValue)]
    struct PyItertoolsCombinations {
        pool: Vec<PyObjectRef>,
        indices: PyRwLock<Vec<usize>>,
        r: AtomicCell<usize>,
        exhausted: AtomicCell<bool>,
    }

    #[derive(FromArgs)]
    struct CombinationsNewArgs {
        #[pyarg(positional)]
        iterable: PyObjectRef,
        #[pyarg(positional)]
        r: PyIntRef,
    }

    impl SlotConstructor for PyItertoolsCombinations {
        type Args = CombinationsNewArgs;

        fn py_new(
            cls: PyTypeRef,
            Self::Args { iterable, r }: Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult {
            let pool = vm.extract_elements(&iterable)?;

            let r = r.as_bigint();
            if r.is_negative() {
                return Err(vm.new_value_error("r must be non-negative".to_owned()));
            }
            let r = r.to_usize().unwrap();

            let n = pool.len();

            PyItertoolsCombinations {
                pool,
                indices: PyRwLock::new((0..r).collect()),
                r: AtomicCell::new(r),
                exhausted: AtomicCell::new(r > n),
            }
            .into_pyresult_with_type(vm, cls)
        }
    }

    #[pyimpl(with(PyIter, SlotConstructor))]
    impl PyItertoolsCombinations {}
    impl PyIter for PyItertoolsCombinations {
        fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            // stop signal
            if zelf.exhausted.load() {
                return Err(vm.new_stop_iteration());
            }

            let n = zelf.pool.len();
            let r = zelf.r.load();

            if r == 0 {
                zelf.exhausted.store(true);
                return Ok(vm.ctx.new_tuple(vec![]));
            }

            let res = vm.ctx.new_tuple(
                zelf.indices
                    .read()
                    .iter()
                    .map(|&i| zelf.pool[i].clone())
                    .collect(),
            );

            let mut indices = zelf.indices.write();

            // Scan indices right-to-left until finding one that is not at its maximum (i + n - r).
            let mut idx = r as isize - 1;
            while idx >= 0 && indices[idx as usize] == idx as usize + n - r {
                idx -= 1;
            }

            // If no suitable index is found, then the indices are all at
            // their maximum value and we're done.
            if idx < 0 {
                zelf.exhausted.store(true);
            } else {
                // Increment the current index which we know is not at its
                // maximum.  Then move back to the right setting each index
                // to its lowest possible value (one higher than the index
                // to its left -- this maintains the sort order invariant).
                indices[idx as usize] += 1;
                for j in idx as usize + 1..r {
                    indices[j] = indices[j - 1] + 1;
                }
            }

            Ok(res)
        }
    }

    #[pyattr]
    #[pyclass(name = "combinations_with_replacement")]
    #[derive(Debug, PyValue)]
    struct PyItertoolsCombinationsWithReplacement {
        pool: Vec<PyObjectRef>,
        indices: PyRwLock<Vec<usize>>,
        r: AtomicCell<usize>,
        exhausted: AtomicCell<bool>,
    }

    impl SlotConstructor for PyItertoolsCombinationsWithReplacement {
        type Args = CombinationsNewArgs;

        fn py_new(
            cls: PyTypeRef,
            Self::Args { iterable, r }: Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult {
            let pool = vm.extract_elements(&iterable)?;
            let r = r.as_bigint();
            if r.is_negative() {
                return Err(vm.new_value_error("r must be non-negative".to_owned()));
            }
            let r = r.to_usize().unwrap();

            let n = pool.len();

            PyItertoolsCombinationsWithReplacement {
                pool,
                indices: PyRwLock::new(vec![0; r]),
                r: AtomicCell::new(r),
                exhausted: AtomicCell::new(n == 0 && r > 0),
            }
            .into_pyresult_with_type(vm, cls)
        }
    }

    #[pyimpl(with(PyIter, SlotConstructor))]
    impl PyItertoolsCombinationsWithReplacement {}

    impl PyIter for PyItertoolsCombinationsWithReplacement {
        fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            // stop signal
            if zelf.exhausted.load() {
                return Err(vm.new_stop_iteration());
            }

            let n = zelf.pool.len();
            let r = zelf.r.load();

            if r == 0 {
                zelf.exhausted.store(true);
                return Ok(vm.ctx.new_tuple(vec![]));
            }

            let mut indices = zelf.indices.write();

            let res = vm
                .ctx
                .new_tuple(indices.iter().map(|&i| zelf.pool[i].clone()).collect());

            // Scan indices right-to-left until finding one that is not at its maximum (i + n - r).
            let mut idx = r as isize - 1;
            while idx >= 0 && indices[idx as usize] == n - 1 {
                idx -= 1;
            }

            // If no suitable index is found, then the indices are all at
            // their maximum value and we're done.
            if idx < 0 {
                zelf.exhausted.store(true);
            } else {
                let index = indices[idx as usize] + 1;

                // Increment the current index which we know is not at its
                // maximum. Then set all to the right to the same value.
                for j in idx as usize..r {
                    indices[j as usize] = index as usize;
                }
            }

            Ok(res)
        }
    }

    #[pyattr]
    #[pyclass(name = "permutations")]
    #[derive(Debug, PyValue)]
    struct PyItertoolsPermutations {
        pool: Vec<PyObjectRef>,               // Collected input iterable
        indices: PyRwLock<Vec<usize>>,        // One index per element in pool
        cycles: PyRwLock<Vec<usize>>,         // One rollover counter per element in the result
        result: PyRwLock<Option<Vec<usize>>>, // Indexes of the most recently returned result
        r: AtomicCell<usize>,                 // Size of result tuple
        exhausted: AtomicCell<bool>,          // Set when the iterator is exhausted
    }

    #[derive(FromArgs)]
    struct PermutationsNewArgs {
        #[pyarg(positional)]
        iterable: PyObjectRef,
        #[pyarg(positional, optional)]
        r: OptionalOption<PyObjectRef>,
    }

    impl SlotConstructor for PyItertoolsPermutations {
        type Args = PermutationsNewArgs;

        fn py_new(
            cls: PyTypeRef,
            Self::Args { iterable, r }: Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult {
            let pool = vm.extract_elements(&iterable)?;

            let n = pool.len();
            // If r is not provided, r == n. If provided, r must be a positive integer, or None.
            // If None, it behaves the same as if it was not provided.
            let r = match r.flatten() {
                Some(r) => {
                    let val = r
                        .payload::<PyInt>()
                        .ok_or_else(|| vm.new_type_error("Expected int as r".to_owned()))?
                        .as_bigint();

                    if val.is_negative() {
                        return Err(vm.new_value_error("r must be non-negative".to_owned()));
                    }
                    val.to_usize().unwrap()
                }
                None => n,
            };

            PyItertoolsPermutations {
                pool,
                indices: PyRwLock::new((0..n).collect()),
                cycles: PyRwLock::new((0..r.min(n)).map(|i| n - i).collect()),
                result: PyRwLock::new(None),
                r: AtomicCell::new(r),
                exhausted: AtomicCell::new(r > n),
            }
            .into_pyresult_with_type(vm, cls)
        }
    }

    #[pyimpl(with(PyIter, SlotConstructor))]
    impl PyItertoolsPermutations {}
    impl PyIter for PyItertoolsPermutations {
        fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            // stop signal
            if zelf.exhausted.load() {
                return Err(vm.new_stop_iteration());
            }

            let n = zelf.pool.len();
            let r = zelf.r.load();

            if n == 0 {
                zelf.exhausted.store(true);
                return Ok(vm.ctx.new_tuple(vec![]));
            }

            let mut result = zelf.result.write();

            if let Some(ref mut result) = *result {
                let mut indices = zelf.indices.write();
                let mut cycles = zelf.cycles.write();
                let mut sentinel = false;

                // Decrement rightmost cycle, moving leftward upon zero rollover
                for i in (0..r).rev() {
                    cycles[i] -= 1;

                    if cycles[i] == 0 {
                        // rotation: indices[i:] = indices[i+1:] + indices[i:i+1]
                        let index = indices[i];
                        for j in i..n - 1 {
                            indices[j] = indices[j + 1];
                        }
                        indices[n - 1] = index;
                        cycles[i] = n - i;
                    } else {
                        let j = cycles[i];
                        indices.swap(i, n - j);

                        for k in i..r {
                            // start with i, the leftmost element that changed
                            // yield tuple(pool[k] for k in indices[:r])
                            result[k] = indices[k];
                        }
                        sentinel = true;
                        break;
                    }
                }
                if !sentinel {
                    zelf.exhausted.store(true);
                    return Err(vm.new_stop_iteration());
                }
            } else {
                // On the first pass, initialize result tuple using the indices
                *result = Some((0..r).collect());
            }

            Ok(vm.ctx.new_tuple(
                result
                    .as_ref()
                    .unwrap()
                    .iter()
                    .map(|&i| zelf.pool[i].clone())
                    .collect(),
            ))
        }
    }

    #[derive(FromArgs)]
    struct ZipLongestArgs {
        #[pyarg(named, optional)]
        fillvalue: OptionalArg<PyObjectRef>,
    }

    impl SlotConstructor for PyItertoolsZipLongest {
        type Args = (Args<PyObjectRef>, ZipLongestArgs);

        fn py_new(cls: PyTypeRef, (iterables, args): Self::Args, vm: &VirtualMachine) -> PyResult {
            let fillvalue = args.fillvalue.unwrap_or_none(vm);
            let iterators = iterables
                .into_iter()
                .map(|iterable| get_iter(vm, iterable))
                .collect::<Result<Vec<_>, _>>()?;

            PyItertoolsZipLongest {
                iterators,
                fillvalue,
            }
            .into_pyresult_with_type(vm, cls)
        }
    }

    #[pyattr]
    #[pyclass(name = "zip_longest")]
    #[derive(Debug, PyValue)]
    struct PyItertoolsZipLongest {
        iterators: Vec<PyObjectRef>,
        fillvalue: PyObjectRef,
    }

    #[pyimpl(with(PyIter, SlotConstructor))]
    impl PyItertoolsZipLongest {}
    impl PyIter for PyItertoolsZipLongest {
        fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            if zelf.iterators.is_empty() {
                Err(vm.new_stop_iteration())
            } else {
                let mut result: Vec<PyObjectRef> = Vec::new();
                let mut numactive = zelf.iterators.len();

                for idx in 0..zelf.iterators.len() {
                    let next_obj = match call_next(vm, &zelf.iterators[idx]) {
                        Ok(obj) => obj,
                        Err(err) => {
                            if !err.isinstance(&vm.ctx.exceptions.stop_iteration) {
                                return Err(err);
                            }
                            numactive -= 1;
                            if numactive == 0 {
                                return Err(vm.new_stop_iteration());
                            }
                            zelf.fillvalue.clone()
                        }
                    };
                    result.push(next_obj);
                }
                Ok(vm.ctx.new_tuple(result))
            }
        }
    }

    #[pyattr]
    #[pyclass(name = "pairwise")]
    #[derive(Debug, PyValue)]
    struct PyItertoolsPairwise {
        iterator: PyObjectRef,
        old: PyRwLock<Option<PyObjectRef>>,
    }

    impl SlotConstructor for PyItertoolsPairwise {
        type Args = PyObjectRef;

        fn py_new(cls: PyTypeRef, iterable: Self::Args, vm: &VirtualMachine) -> PyResult {
            let iterator = get_iter(vm, iterable)?;

            PyItertoolsPairwise {
                iterator,
                old: PyRwLock::new(None),
            }
            .into_pyresult_with_type(vm, cls)
        }
    }

    #[pyimpl(with(PyIter, SlotConstructor))]
    impl PyItertoolsPairwise {}
    impl PyIter for PyItertoolsPairwise {
        fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            let old = match zelf.old.read().clone() {
                None => call_next(vm, &zelf.iterator)?,
                Some(obj) => obj,
            };
            let new = call_next(vm, &zelf.iterator)?;
            *zelf.old.write() = Some(new.clone());
            Ok(vm.ctx.new_tuple(vec![old, new]))
        }
    }
}
