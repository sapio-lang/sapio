from functools import singledispatch, update_wrapper
# shim till python 3.8
# see https://stackoverflow.com/questions/24601722/how-can-i-use-functools-singledispatch-with-instance-methods
def methdispatch(func):
    dispatcher = singledispatch(func)

    def wrapper(*args, **kw):
        return dispatcher.dispatch(args[1].__class__)(*args, **kw)

    wrapper.register = dispatcher.register
    update_wrapper(wrapper, func)
    return wrapper
