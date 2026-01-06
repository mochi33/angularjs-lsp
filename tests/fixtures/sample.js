// Sample AngularJS application
angular.module('myApp', ['ngRoute', 'myApp.services'])

.controller('MainController', ['$scope', 'UserService', function($scope, UserService) {
    $scope.users = [];

    $scope.loadUsers = function() {
        UserService.getAll().then(function(users) {
            $scope.users = users;
        });
    };
}])

.controller('DetailController', ['$scope', '$routeParams', 'UserService', function($scope, $routeParams, UserService) {
    $scope.user = null;

    UserService.getById($routeParams.id).then(function(user) {
        $scope.user = user;
    });
}])

.service('UserService', ['$http', '$q', function($http, $q) {
    this.getAll = function() {
        return $http.get('/api/users').then(function(response) {
            return response.data;
        });
    };

    this.getById = function(id) {
        return $http.get('/api/users/' + id).then(function(response) {
            return response.data;
        });
    };
}])

.directive('userCard', ['UserService', function(UserService) {
    return {
        restrict: 'E',
        scope: {
            userId: '='
        },
        template: '<div class="user-card">{{ user.name }}</div>',
        link: function(scope) {
            UserService.getById(scope.userId).then(function(user) {
                scope.user = user;
            });
        }
    };
}])

.factory('AuthService', ['$http', '$q', function($http, $q) {
    return {
        login: function(credentials) {
            return $http.post('/api/login', credentials);
        },
        logout: function() {
            return $http.post('/api/logout');
        }
    };
}])

.constant('API_URL', 'https://api.example.com')

.value('appConfig', {
    debug: true,
    version: '1.0.0'
});

// $inject pattern
function AdminController($scope, UserService, AuthService) {
    $scope.users = [];

    $scope.loadUsers = function() {
        if (AuthService.isLoggedIn()) {
            UserService.getAll().then(function(users) {
                $scope.users = users;
            });
        }
    };
}
AdminController.$inject = ['$scope', 'UserService', 'AuthService'];

angular.module('myApp').controller('AdminController', AdminController);

// Another $inject pattern with variable
var SettingsService = function($http, API_URL) {
    this.getSettings = function() {
        return $http.get(API_URL + '/settings');
    };
};
SettingsService.$inject = ['$http', 'API_URL'];

angular.module('myApp').service('SettingsService', SettingsService);
